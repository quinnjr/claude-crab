/*
 * SPDX-License-Identifier: MIT
 */

#include "SessionTracker.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

namespace
{
QJsonObject ev(const QString &session, const QString &event, const QString &tool = {})
{
    QJsonObject o{
        {QStringLiteral("session_id"), session},
        {QStringLiteral("hook_event_name"), event},
    };
    if (!tool.isEmpty()) {
        o.insert(QStringLiteral("tool_name"), tool);
    }
    return o;
}
}

class TestSessionTracker : public QObject
{
    Q_OBJECT

private Q_SLOTS:
    void singleSessionHappyPath();
    void toolNameTracked();
    void notificationWinsOverWorking();
    void multipleSessionsAggregate_data();
    void multipleSessionsAggregate();
    void sessionEndDeregisters();
    void unknownSessionIsRegisteredImplicitly();
    void duplicateAndOutOfOrderEvents();
    void unknownEventKeepsStateButRefreshesLiveness();
    void staleSweepRetiresSilentSession();
    void staleSweepKeepsRecentSession();
    void stopEmitsFinished();
    void failedToolEmitsErrored_data();
    void failedToolEmitsErrored();
    void aggregateChangedNotSpammed();

    // Filesystem path
    void drainsInboxInTimestampOrder();
    void survivesMalformedFiles();
    void respectsPerPollCap();
};

void TestSessionTracker::singleSessionHappyPath()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    QCOMPARE(t.aggregateState(), SessionTracker::Idle);

    t.handleEvent(ev("a", "SessionStart"), 0);
    QCOMPARE(t.aggregateState(), SessionTracker::Idle);

    t.handleEvent(ev("a", "UserPromptSubmit"), 1);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);

    t.handleEvent(ev("a", "PreToolUse", "Bash"), 2);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);
    QCOMPARE(t.currentTool(), QStringLiteral("Bash"));

    t.handleEvent(ev("a", "PostToolUse", "Bash"), 3);
    QCOMPARE(t.currentTool(), QString());

    t.handleEvent(ev("a", "Stop"), 4);
    QCOMPARE(t.aggregateState(), SessionTracker::Idle);
}

void TestSessionTracker::toolNameTracked()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.handleEvent(ev("a", "PreToolUse", "Edit"), 0);
    QCOMPARE(t.currentTool(), QStringLiteral("Edit"));
    t.handleEvent(ev("a", "PreToolUse", "Grep"), 1);
    QCOMPARE(t.currentTool(), QStringLiteral("Grep"));
}

void TestSessionTracker::notificationWinsOverWorking()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.handleEvent(ev("a", "PreToolUse", "Bash"), 0);
    t.handleEvent(ev("b", "Notification"), 1);
    QCOMPARE(t.aggregateState(), SessionTracker::WaitingInput);
    // A tool name would be misleading while we are blocked on the user.
    QCOMPARE(t.currentTool(), QString());
}

void TestSessionTracker::multipleSessionsAggregate_data()
{
    QTest::addColumn<QStringList>("events");
    QTest::addColumn<int>("expected");

    QTest::newRow("all idle") << QStringList{"a:Stop", "b:Stop"} << int(SessionTracker::Idle);
    QTest::newRow("one working")
        << QStringList{"a:Stop", "b:UserPromptSubmit"} << int(SessionTracker::Working);
    QTest::newRow("four interleaved")
        << QStringList{"a:UserPromptSubmit", "b:UserPromptSubmit", "c:Stop", "d:UserPromptSubmit"}
        << int(SessionTracker::Working);
    QTest::newRow("one waiting beats three working")
        << QStringList{"a:UserPromptSubmit", "b:UserPromptSubmit", "c:Notification",
                       "d:UserPromptSubmit"}
        << int(SessionTracker::WaitingInput);
}

void TestSessionTracker::multipleSessionsAggregate()
{
    QFETCH(QStringList, events);
    QFETCH(int, expected);

    SessionTracker t(QStringLiteral("/nonexistent"));
    qint64 clock = 0;
    for (const QString &spec : events) {
        const QStringList parts = spec.split(QLatin1Char(':'));
        t.handleEvent(ev(parts.at(0), parts.at(1)), ++clock);
    }
    QCOMPARE(int(t.aggregateState()), expected);
}

void TestSessionTracker::sessionEndDeregisters()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.handleEvent(ev("a", "UserPromptSubmit"), 0);
    t.handleEvent(ev("b", "UserPromptSubmit"), 1);
    QCOMPARE(t.activeSessions(), 2);

    t.handleEvent(ev("a", "SessionEnd"), 2);
    QCOMPARE(t.activeSessions(), 1);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);

    t.handleEvent(ev("b", "SessionEnd"), 3);
    QCOMPARE(t.activeSessions(), 0);
    QCOMPARE(t.aggregateState(), SessionTracker::Idle);
}

void TestSessionTracker::unknownSessionIsRegisteredImplicitly()
{
    // The crab may be started after sessions are already running.
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.handleEvent(ev("never-announced", "PreToolUse", "Read"), 0);
    QCOMPARE(t.activeSessions(), 1);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);
}

void TestSessionTracker::duplicateAndOutOfOrderEvents()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.handleEvent(ev("a", "Stop"), 0);
    t.handleEvent(ev("a", "Stop"), 1);
    QCOMPARE(t.aggregateState(), SessionTracker::Idle);

    // PostToolUse arriving after Stop must not resurrect a finished session as
    // anything other than working -- last event wins, by design.
    t.handleEvent(ev("a", "PostToolUse", "Bash"), 2);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);
}

void TestSessionTracker::unknownEventKeepsStateButRefreshesLiveness()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.setStaleTimeoutMs(100);
    t.handleEvent(ev("a", "UserPromptSubmit"), 1000);
    t.handleEvent(ev("a", "SubagentStop"), 1050);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);

    t.sweepStale(1100); // within timeout of the SubagentStop
    QCOMPARE(t.activeSessions(), 1);
}

void TestSessionTracker::staleSweepRetiresSilentSession()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.setStaleTimeoutMs(1000);
    t.handleEvent(ev("a", "UserPromptSubmit"), 0);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);

    QSignalSpy spy(&t, &SessionTracker::aggregateChanged);
    t.sweepStale(5000);
    QCOMPARE(t.activeSessions(), 0);
    QCOMPARE(t.aggregateState(), SessionTracker::Idle);
    QCOMPARE(spy.count(), 1);
}

void TestSessionTracker::staleSweepKeepsRecentSession()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.setStaleTimeoutMs(1000);
    t.handleEvent(ev("a", "UserPromptSubmit"), 4500);
    t.sweepStale(5000);
    QCOMPARE(t.activeSessions(), 1);
    QCOMPARE(t.aggregateState(), SessionTracker::Working);
}

void TestSessionTracker::stopEmitsFinished()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    QSignalSpy spy(&t, &SessionTracker::finished);
    t.handleEvent(ev("a", "UserPromptSubmit"), 0);
    QCOMPARE(spy.count(), 0);
    t.handleEvent(ev("a", "Stop"), 1);
    QCOMPARE(spy.count(), 1);
}

void TestSessionTracker::failedToolEmitsErrored_data()
{
    QTest::addColumn<QJsonValue>("response");
    QTest::addColumn<bool>("expected");

    QTest::newRow("success false") << QJsonValue(QJsonObject{{"success", false}}) << true;
    QTest::newRow("success true") << QJsonValue(QJsonObject{{"success", true}}) << false;
    QTest::newRow("error key") << QJsonValue(QJsonObject{{"error", "boom"}}) << true;
    QTest::newRow("plain object") << QJsonValue(QJsonObject{{"stdout", "hi"}}) << false;
    QTest::newRow("error string") << QJsonValue(QStringLiteral("Error: no such file")) << true;
    QTest::newRow("ok string") << QJsonValue(QStringLiteral("done")) << false;
    QTest::newRow("absent") << QJsonValue() << false;
}

void TestSessionTracker::failedToolEmitsErrored()
{
    QFETCH(QJsonValue, response);
    QFETCH(bool, expected);

    SessionTracker t(QStringLiteral("/nonexistent"));
    QSignalSpy spy(&t, &SessionTracker::errored);

    QJsonObject payload = ev("a", "PostToolUse", "Bash");
    if (!response.isUndefined()) {
        payload.insert(QStringLiteral("tool_response"), response);
    }
    t.handleEvent(payload, 0);

    QCOMPARE(spy.count(), expected ? 1 : 0);
}

void TestSessionTracker::aggregateChangedNotSpammed()
{
    SessionTracker t(QStringLiteral("/nonexistent"));
    t.handleEvent(ev("a", "PreToolUse", "Bash"), 0);
    QSignalSpy spy(&t, &SessionTracker::aggregateChanged);
    // Same session, same tool, same state: nothing observable changed.
    t.handleEvent(ev("a", "PreToolUse", "Bash"), 1);
    QCOMPARE(spy.count(), 0);
}

// --- filesystem path -------------------------------------------------------

static void writeEvent(const QDir &dir, const QString &name, const QByteArray &body)
{
    QFile f(dir.filePath(name));
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write(body);
}

void TestSessionTracker::drainsInboxInTimestampOrder()
{
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QDir dir(tmp.path());

    // Written out of order on purpose; the name sort must fix it.
    writeEvent(dir, QStringLiteral("200.json"),
               QJsonDocument(ev("a", "Stop")).toJson(QJsonDocument::Compact));
    writeEvent(dir, QStringLiteral("100.json"),
               QJsonDocument(ev("a", "UserPromptSubmit")).toJson(QJsonDocument::Compact));

    SessionTracker t(tmp.path());
    t.poll();

    QCOMPARE(t.aggregateState(), SessionTracker::Idle); // Stop applied last
    QCOMPARE(QDir(tmp.path()).entryList({QStringLiteral("*.json")}, QDir::Files).size(), 0);
}

void TestSessionTracker::survivesMalformedFiles()
{
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QDir dir(tmp.path());

    writeEvent(dir, QStringLiteral("100.json"), "{ not json");
    writeEvent(dir, QStringLiteral("110.json"), "");
    writeEvent(dir, QStringLiteral("120.json"), "[1,2,3]"); // valid JSON, wrong shape
    writeEvent(dir, QStringLiteral("130.json"), "{\"hook_event_name\":\"Stop\"}"); // no id
    writeEvent(dir, QStringLiteral("140.json"),
               QJsonDocument(ev("a", "UserPromptSubmit")).toJson(QJsonDocument::Compact));

    SessionTracker t(tmp.path());
    t.poll();

    QCOMPARE(t.aggregateState(), SessionTracker::Working);
    // Everything is consumed, including the junk, so it cannot accumulate.
    QCOMPARE(QDir(tmp.path()).entryList({QStringLiteral("*.json")}, QDir::Files).size(), 0);
}

void TestSessionTracker::respectsPerPollCap()
{
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QDir dir(tmp.path());

    const int total = SessionTracker::MaxFilesPerPoll + 10;
    for (int i = 0; i < total; ++i) {
        writeEvent(dir, QStringLiteral("%1.json").arg(i, 6, 10, QLatin1Char('0')),
                   QJsonDocument(ev("a", "UserPromptSubmit")).toJson(QJsonDocument::Compact));
    }

    SessionTracker t(tmp.path());
    t.poll();
    QCOMPARE(QDir(tmp.path()).entryList({QStringLiteral("*.json")}, QDir::Files).size(), 10);

    t.poll();
    QCOMPARE(QDir(tmp.path()).entryList({QStringLiteral("*.json")}, QDir::Files).size(), 0);
}

QTEST_MAIN(TestSessionTracker)
#include "tst_sessiontracker.moc"
