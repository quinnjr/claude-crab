/*
 * SPDX-License-Identifier: MIT
 */

#pragma once

#include <QElapsedTimer>
#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>

class QFileSystemWatcher;
class QTimer;

/**
 * Turns a stream of Claude Code hook payloads into a single aggregate state for
 * the crab to animate.
 *
 * Events arrive as one JSON file per hook invocation in @c inboxDir, written by
 * the shell snippet that crab_hooks.py registers. Files are consumed in filename
 * order (nanosecond timestamps) and unlinked.
 *
 * This class deliberately knows nothing about QML, Wayland, or rendering, so the
 * whole state machine is unit-testable by feeding it recorded payloads.
 */
class SessionTracker : public QObject
{
    Q_OBJECT
    Q_PROPERTY(State aggregateState READ aggregateState NOTIFY aggregateChanged)
    Q_PROPERTY(QString currentTool READ currentTool NOTIFY aggregateChanged)
    Q_PROPERTY(int activeSessions READ activeSessions NOTIFY aggregateChanged)

public:
    enum State {
        Idle,
        Working,
        WaitingInput,
    };
    Q_ENUM(State)

    explicit SessionTracker(QString inboxDir, QObject *parent = nullptr);
    ~SessionTracker() override;

    State aggregateState() const
    {
        return m_aggregate;
    }
    QString currentTool() const
    {
        return m_currentTool;
    }
    int activeSessions() const
    {
        return int(m_sessions.size());
    }

    /** Sessions silent for longer than this are retired. Default 10 minutes. */
    void setStaleTimeoutMs(qint64 ms)
    {
        m_staleTimeoutMs = ms;
    }
    qint64 staleTimeoutMs() const
    {
        return m_staleTimeoutMs;
    }

    /** Start watching the inbox. Separate from the constructor so tests can
     *  drive the state machine without touching the filesystem. */
    void start();

    /** Drain the inbox now. Safe to call at any time. */
    Q_INVOKABLE void poll();

    /** Apply one hook payload. @p nowMs is injected so tests are deterministic. */
    void handleEvent(const QJsonObject &payload, qint64 nowMs);

    /** Retire sessions with no event since @p nowMs - staleTimeoutMs. */
    void sweepStale(qint64 nowMs);

    /**
     * Enforce the inbox budget: drop events older than @p maxAgeMs, then drop
     * the oldest remaining until the directory is under @p maxBytes.
     *
     * The hooks keep writing whether or not the crab is running, so without
     * this the inbox grows without limit across a logged-out weekend. Stale
     * events are worthless anyway -- a session state from an hour ago says
     * nothing about now.
     */
    void setInboxBudget(qint64 maxBytes, qint64 maxAgeMs);
    void pruneInbox(qint64 nowMs);

    /**
     * Recover the fields the crab needs from a payload the hook truncated.
     * Returns an empty object if @p raw does not yield a usable event.
     */
    static QJsonObject salvage(const QByteArray &raw);

    /** At most this many files are drained per poll, oldest first. */
    static constexpr int MaxFilesPerPoll = 200;

Q_SIGNALS:
    void aggregateChanged();
    /** A session reached Stop: play the celebration once. */
    void finished();
    /** A tool reported failure: play the stumble once. */
    void errored();

private:
    struct Session {
        State state = Idle;
        QString tool;
        qint64 lastSeenMs = 0;
    };

    void recompute();
    static bool responseIndicatesError(const QJsonValue &response);

    QString m_inboxDir;
    QHash<QString, Session> m_sessions;
    QHash<QString, qint64> m_order; // session id -> last update, for tool priority

    State m_aggregate = Idle;
    QString m_currentTool;
    qint64 m_staleTimeoutMs = 10 * 60 * 1000;
    qint64 m_inboxMaxBytes = 32LL * 1024 * 1024;
    qint64 m_inboxMaxAgeMs = 60 * 60 * 1000;
    qint64 m_sequence = 0;
    int m_lastCount = 0;

    QFileSystemWatcher *m_watcher = nullptr;
    QTimer *m_safetyPoll = nullptr;
    QTimer *m_sweepTimer = nullptr;
};
