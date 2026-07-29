/*
 * SPDX-License-Identifier: MIT
 */

#include "CrabConfig.h"

#include <QDir>
#include <QTemporaryDir>
#include <QTest>

class TestCrabConfig : public QObject
{
    Q_OBJECT

private Q_SLOTS:
    void missingFileYieldsDefaults();
    void malformedFileYieldsDefaults();
    void readsEveryKey();
    void unknownSpriteFallsBackToDefault();
    void spriteFileNameMatchesVariant_data();
    void spriteFileNameMatchesVariant();
    void invalidSleepCornerFallsBack();
    void tinyStripHeightFallsBack();
    void unknownReactionIsIgnored();

    void inboxDirHonoursExplicitOverride();
    void inboxDirUsesXdgStateHomeOnTheHost();
    void inboxDirIgnoresXdgStateHomeInsideFlatpak();
    void inboxDirFallsBackToTheConventionalPath();
    void inboxDirOverrideBeatsFlatpakDetection();

private:
    QString write(const QByteArray &json);
    QTemporaryDir m_dir;
    int m_counter = 0;
};

QString TestCrabConfig::write(const QByteArray &json)
{
    const QString path = m_dir.filePath(QStringLiteral("cfg%1.json").arg(m_counter++));
    QFile f(path);
    if (!f.open(QIODevice::WriteOnly)) {
        return {};
    }
    f.write(json);
    return path;
}

void TestCrabConfig::missingFileYieldsDefaults()
{
    const CrabConfig config = CrabConfig::load(m_dir.filePath(QStringLiteral("nope.json")));
    QCOMPARE(config.stripHeight, 72);
    QCOMPARE(config.sprite, QStringLiteral("default"));
    QCOMPARE(config.sleepCorner, QStringLiteral("right"));
}

void TestCrabConfig::malformedFileYieldsDefaults()
{
    // A config typo must not cost you the crab.
    const CrabConfig config = CrabConfig::load(write("{ not json"));
    QCOMPARE(config.stripHeight, 72);
    QCOMPARE(config.sprite, QStringLiteral("default"));
}

void TestCrabConfig::readsEveryKey()
{
    const CrabConfig config = CrabConfig::load(write(R"({
        "stripHeight": 96,
        "crabScale": 1.5,
        "output": "DP-3",
        "sleepCorner": "left",
        "sprite": "fancy",
        "staleTimeoutMinutes": 3,
        "reactions": { "error": false }
    })"));

    QCOMPARE(config.stripHeight, 96);
    QCOMPARE(config.crabScale, 1.5);
    QCOMPARE(config.output, QStringLiteral("DP-3"));
    QCOMPARE(config.sleepCorner, QStringLiteral("left"));
    QCOMPARE(config.sprite, QStringLiteral("fancy"));
    QCOMPARE(config.staleTimeoutMinutes, 3);
    QCOMPARE(config.reactions.value(QStringLiteral("error")).toBool(), false);
    // Unlisted reactions keep their defaults rather than being cleared.
    QCOMPARE(config.reactions.value(QStringLiteral("waiting")).toBool(), true);
}

void TestCrabConfig::unknownSpriteFallsBackToDefault()
{
    const CrabConfig config = CrabConfig::load(write(R"({"sprite": "sombrero"})"));
    QCOMPARE(config.sprite, QStringLiteral("default"));
    QCOMPARE(config.spriteFileName(), QStringLiteral("spritesheet.png"));
}

void TestCrabConfig::spriteFileNameMatchesVariant_data()
{
    QTest::addColumn<QString>("variant");
    QTest::addColumn<QString>("filename");

    QTest::newRow("default") << "default" << "spritesheet.png";
    QTest::newRow("fancy") << "fancy" << "spritesheet-fancy.png";
    QTest::newRow("party") << "party" << "spritesheet-party.png";
}

void TestCrabConfig::spriteFileNameMatchesVariant()
{
    QFETCH(QString, variant);
    QFETCH(QString, filename);

    // Every declared variant must map to a distinct sheet, or selecting one
    // would silently render another.
    QVERIFY(CrabConfig::spriteVariants().contains(variant));

    CrabConfig config;
    config.sprite = variant;
    QCOMPARE(config.spriteFileName(), filename);
}

void TestCrabConfig::invalidSleepCornerFallsBack()
{
    const CrabConfig config = CrabConfig::load(write(R"({"sleepCorner": "middle"})"));
    QCOMPARE(config.sleepCorner, QStringLiteral("right"));
}

void TestCrabConfig::tinyStripHeightFallsBack()
{
    const CrabConfig config = CrabConfig::load(write(R"({"stripHeight": 2})"));
    QCOMPARE(config.stripHeight, 72);
}

void TestCrabConfig::unknownReactionIsIgnored()
{
    const CrabConfig config = CrabConfig::load(write(R"({"reactions": {"nonsense": false}})"));
    QVERIFY(!config.reactions.contains(QStringLiteral("nonsense")));
    QCOMPARE(config.reactions.size(), 4);
}

// --- inbox path ------------------------------------------------------------
//
// The hooks always write to the host's state directory, because Claude Code is
// not sandboxed. Getting this wrong inside a Flatpak leaves the crab watching
// an empty directory forever, with no error to explain it.

void TestCrabConfig::inboxDirHonoursExplicitOverride()
{
    qputenv("CLAUDE_CRAB_STATE_DIR", "/somewhere/else");
    QCOMPARE(CrabConfig::inboxDir(false), QStringLiteral("/somewhere/else/inbox"));
    qunsetenv("CLAUDE_CRAB_STATE_DIR");
}

void TestCrabConfig::inboxDirUsesXdgStateHomeOnTheHost()
{
    qunsetenv("CLAUDE_CRAB_STATE_DIR");
    qputenv("XDG_STATE_HOME", "/custom/state");
    QCOMPARE(CrabConfig::inboxDir(false), QStringLiteral("/custom/state/claude-crab/inbox"));
    qunsetenv("XDG_STATE_HOME");
}

void TestCrabConfig::inboxDirIgnoresXdgStateHomeInsideFlatpak()
{
    qunsetenv("CLAUDE_CRAB_STATE_DIR");
    // What Flatpak actually sets: the sandbox's private state directory.
    qputenv("XDG_STATE_HOME", "/home/u/.var/app/dev.quinnjr.claude-crab/.local/state");

    const QString dir = CrabConfig::inboxDir(true);
    QVERIFY2(!dir.contains(QLatin1String(".var/app")), qPrintable(dir));
    QCOMPARE(dir, QDir::homePath() + QStringLiteral("/.local/state/claude-crab/inbox"));

    qunsetenv("XDG_STATE_HOME");
}

void TestCrabConfig::inboxDirFallsBackToTheConventionalPath()
{
    qunsetenv("CLAUDE_CRAB_STATE_DIR");
    qunsetenv("XDG_STATE_HOME");
    QCOMPARE(CrabConfig::inboxDir(false),
             QDir::homePath() + QStringLiteral("/.local/state/claude-crab/inbox"));
}

void TestCrabConfig::inboxDirOverrideBeatsFlatpakDetection()
{
    // The escape hatch for a host with a non-default XDG_STATE_HOME, which a
    // sandboxed process has no way to discover on its own.
    qputenv("CLAUDE_CRAB_STATE_DIR", "/host/state/claude-crab");
    QCOMPARE(CrabConfig::inboxDir(true), QStringLiteral("/host/state/claude-crab/inbox"));
    qunsetenv("CLAUDE_CRAB_STATE_DIR");
}

QTEST_MAIN(TestCrabConfig)
#include "tst_crabconfig.moc"
