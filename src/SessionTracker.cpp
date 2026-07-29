/*
 * SPDX-License-Identifier: MIT
 */

#include "SessionTracker.h"

#include <QDateTime>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QFileSystemWatcher>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLoggingCategory>
#include <QRegularExpression>
#include <QTimer>

Q_LOGGING_CATEGORY(CRAB, "claude.crab")

namespace
{
// The watcher can miss events when a directory churns quickly, so a low-rate
// timer backs it up. This is a safety net, not the primary path.
constexpr int SafetyPollIntervalMs = 2000;
constexpr int SweepIntervalMs = 60 * 1000;
}

SessionTracker::SessionTracker(QString inboxDir, QObject *parent)
    : QObject(parent)
    , m_inboxDir(std::move(inboxDir))
{
}

SessionTracker::~SessionTracker() = default;

void SessionTracker::start()
{
    QDir().mkpath(m_inboxDir);

    m_watcher = new QFileSystemWatcher(this);
    if (!m_watcher->addPath(m_inboxDir)) {
        qCWarning(CRAB) << "cannot watch" << m_inboxDir << "- falling back to polling";
    }
    connect(m_watcher, &QFileSystemWatcher::directoryChanged, this, &SessionTracker::poll);

    m_safetyPoll = new QTimer(this);
    m_safetyPoll->setInterval(SafetyPollIntervalMs);
    connect(m_safetyPoll, &QTimer::timeout, this, &SessionTracker::poll);
    m_safetyPoll->start();

    m_sweepTimer = new QTimer(this);
    m_sweepTimer->setInterval(SweepIntervalMs);
    connect(m_sweepTimer, &QTimer::timeout, this, [this] {
        const qint64 now = QDateTime::currentMSecsSinceEpoch();
        sweepStale(now);
        pruneInbox(now);
    });
    m_sweepTimer->start();

    // Prune before the first drain: after a spell with the crab not running
    // there may be a large backlog, all of it too old to mean anything.
    pruneInbox(QDateTime::currentMSecsSinceEpoch());
    poll();
}

void SessionTracker::poll()
{
    QDir dir(m_inboxDir);
    if (!dir.exists()) {
        return;
    }

    // Filenames are nanosecond timestamps, so a name sort is a time sort.
    const QStringList names = dir.entryList({QStringLiteral("*.json")}, QDir::Files, QDir::Name);
    if (names.isEmpty()) {
        return;
    }

    const int limit = qMin(names.size(), MaxFilesPerPoll);
    if (names.size() > MaxFilesPerPoll) {
        qCWarning(CRAB) << "inbox has" << names.size() << "files; draining oldest"
                        << MaxFilesPerPoll << "this tick";
    }

    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    for (int i = 0; i < limit; ++i) {
        const QString path = dir.filePath(names.at(i));

        QFile file(path);
        if (!file.open(QIODevice::ReadOnly)) {
            qCWarning(CRAB) << "cannot read" << path << "- discarding";
            QFile::remove(path);
            continue;
        }
        const QByteArray raw = file.readAll();
        file.close();
        QFile::remove(path);

        QJsonParseError error;
        const QJsonDocument doc = QJsonDocument::fromJson(raw, &error);
        if (error.error == QJsonParseError::NoError && doc.isObject()) {
            handleEvent(doc.object(), now);
            continue;
        }

        // Most likely the hook capped an oversized payload mid-string. The
        // fields the crab needs all precede tool_input, so they survive.
        const QJsonObject salvaged = salvage(raw);
        if (!salvaged.isEmpty()) {
            handleEvent(salvaged, now);
            continue;
        }

        // A malformed payload is never fatal; the crab keeps walking.
        qCWarning(CRAB) << "discarding unparseable event" << names.at(i) << error.errorString();
    }
}

QJsonObject SessionTracker::salvage(const QByteArray &raw)
{
    // First match wins on purpose: these keys appear near the head of the
    // payload, so an occurrence of the same text inside a later tool_input
    // cannot shadow the real one.
    const QString text = QString::fromUtf8(raw);
    const auto field = [&text](const char *key) -> QString {
        // Custom delimiter: the pattern itself ends in )" which would close a
        // plain raw string early.
        const QRegularExpression re(
            QStringLiteral(R"RX("%1"\s*:\s*"([^"]*)")RX").arg(QLatin1String(key)));
        const QRegularExpressionMatch match = re.match(text);
        return match.hasMatch() ? match.captured(1) : QString();
    };

    const QString id = field("session_id");
    const QString event = field("hook_event_name");
    if (id.isEmpty() || event.isEmpty()) {
        return {};
    }

    QJsonObject obj{
        {QStringLiteral("session_id"), id},
        {QStringLiteral("hook_event_name"), event},
    };
    const QString tool = field("tool_name");
    if (!tool.isEmpty()) {
        obj.insert(QStringLiteral("tool_name"), tool);
    }
    // tool_response sits after tool_input and so is the field most likely to
    // have been cut; a truncated payload simply loses the error blip.
    return obj;
}

void SessionTracker::setInboxBudget(qint64 maxBytes, qint64 maxAgeMs)
{
    m_inboxMaxBytes = maxBytes;
    m_inboxMaxAgeMs = maxAgeMs;
}

void SessionTracker::pruneInbox(qint64 nowMs)
{
    QDir dir(m_inboxDir);
    if (!dir.exists()) {
        return;
    }

    // Include .tmp files: a hook killed mid-write leaves one behind forever.
    const QFileInfoList entries =
        dir.entryInfoList({QStringLiteral("*.json"), QStringLiteral("*.tmp")}, QDir::Files,
                          QDir::Name);
    if (entries.isEmpty()) {
        return;
    }

    int removedAged = 0;
    qint64 total = 0;
    QFileInfoList kept;
    kept.reserve(entries.size());

    for (const QFileInfo &info : entries) {
        const qint64 age = nowMs - info.lastModified().toMSecsSinceEpoch();
        if (age > m_inboxMaxAgeMs) {
            if (QFile::remove(info.absoluteFilePath())) {
                ++removedAged;
            }
            continue;
        }
        total += info.size();
        kept.append(info);
    }

    // Oldest first, until the directory is back inside its byte budget.
    int removedForSize = 0;
    for (const QFileInfo &info : std::as_const(kept)) {
        if (total <= m_inboxMaxBytes) {
            break;
        }
        if (QFile::remove(info.absoluteFilePath())) {
            total -= info.size();
            ++removedForSize;
        }
    }

    // Never drop events silently: a gap in the crab's behaviour should always
    // have a line explaining it.
    if (removedAged || removedForSize) {
        qCWarning(CRAB) << "pruned inbox:" << removedAged << "stale," << removedForSize
                        << "over budget;" << total << "bytes remain";
    }
}

bool SessionTracker::responseIndicatesError(const QJsonValue &response)
{
    if (response.isObject()) {
        const QJsonObject obj = response.toObject();
        if (obj.contains(QLatin1String("success"))) {
            return !obj.value(QLatin1String("success")).toBool(true);
        }
        return obj.contains(QLatin1String("error"));
    }
    if (response.isString()) {
        return response.toString().startsWith(QLatin1String("Error"), Qt::CaseInsensitive);
    }
    return false;
}

void SessionTracker::handleEvent(const QJsonObject &payload, qint64 nowMs)
{
    const QString id = payload.value(QLatin1String("session_id")).toString();
    const QString event = payload.value(QLatin1String("hook_event_name")).toString();
    if (id.isEmpty() || event.isEmpty()) {
        qCWarning(CRAB) << "event missing session_id or hook_event_name; ignoring";
        return;
    }

    if (event == QLatin1String("SessionEnd")) {
        if (m_sessions.remove(id) > 0) {
            m_order.remove(id);
            recompute();
        }
        return;
    }

    // An unknown session is registered implicitly, so starting the crab
    // mid-session still produces correct state.
    Session &session = m_sessions[id];
    session.lastSeenMs = nowMs;
    m_order[id] = ++m_sequence;

    bool emitFinished = false;
    bool emitErrored = false;

    if (event == QLatin1String("SessionStart")) {
        session.state = Idle;
        session.tool.clear();
    } else if (event == QLatin1String("UserPromptSubmit")) {
        session.state = Working;
        session.tool.clear();
    } else if (event == QLatin1String("PreToolUse")) {
        session.state = Working;
        session.tool = payload.value(QLatin1String("tool_name")).toString();
    } else if (event == QLatin1String("PostToolUse")) {
        session.state = Working;
        session.tool.clear();
        emitErrored = responseIndicatesError(payload.value(QLatin1String("tool_response")));
    } else if (event == QLatin1String("Notification")) {
        session.state = WaitingInput;
        session.tool.clear();
    } else if (event == QLatin1String("Stop")) {
        session.state = Idle;
        session.tool.clear();
        emitFinished = true;
    } else {
        // SubagentStop, PreCompact and anything added later: refresh liveness
        // only. Reacting to unknown events would make the crab twitchy.
        return;
    }

    recompute();

    if (emitErrored) {
        Q_EMIT errored();
    }
    if (emitFinished) {
        Q_EMIT finished();
    }
}

void SessionTracker::sweepStale(qint64 nowMs)
{
    const qint64 cutoff = nowMs - m_staleTimeoutMs;
    bool removed = false;
    for (auto it = m_sessions.begin(); it != m_sessions.end();) {
        if (it.value().lastSeenMs < cutoff) {
            // A SIGKILLed session never sends Stop or SessionEnd. Without this
            // the crab would walk forever.
            qCDebug(CRAB) << "retiring stale session" << it.key();
            m_order.remove(it.key());
            it = m_sessions.erase(it);
            removed = true;
        } else {
            ++it;
        }
    }
    if (removed) {
        recompute();
    }
}

void SessionTracker::recompute()
{
    State aggregate = Idle;
    QString tool;
    qint64 newestWorking = -1;

    for (auto it = m_sessions.constBegin(); it != m_sessions.constEnd(); ++it) {
        const Session &session = it.value();
        if (session.state == WaitingInput) {
            aggregate = WaitingInput;
        } else if (session.state == Working && aggregate != WaitingInput) {
            aggregate = Working;
        }
        if (session.state == Working) {
            const qint64 seq = m_order.value(it.key(), 0);
            if (seq > newestWorking) {
                newestWorking = seq;
                tool = session.tool;
            }
        }
    }

    if (aggregate != Working) {
        tool.clear();
    }

    const int count = int(m_sessions.size());
    if (aggregate != m_aggregate || tool != m_currentTool || count != m_lastCount) {
        m_aggregate = aggregate;
        m_currentTool = tool;
        m_lastCount = count;
        Q_EMIT aggregateChanged();
    }
}
