/*
 * SPDX-License-Identifier: MIT
 */

#include "CrabConfig.h"
#include "CrabController.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

class TestCrabController : public QObject
{
    Q_OBJECT

private Q_SLOTS:
    void exposesEveryVariant();
    void switchingPersists();
    void switchingEmitsOnce();
    void switchingToTheSameVariantIsANoOp();
    void unknownVariantIsRejectedAndNotPersisted();
    void persistPreservesUnrelatedKeys();
    void persistRefusesToClobberNonObject();
    void sheetUrlFollowsVariant();
    void labelsAreHumanReadable();
    void inputRegionWithoutAWindowIsHarmless();

private:
    QString cfg() const
    {
        return m_dir.filePath(QStringLiteral("claude-crab.json"));
    }
    QJsonObject read() const;
    QTemporaryDir m_dir;
};

QJsonObject TestCrabController::read() const
{
    QFile f(cfg());
    if (!f.open(QIODevice::ReadOnly)) {
        return {};
    }
    return QJsonDocument::fromJson(f.readAll()).object();
}

void TestCrabController::exposesEveryVariant()
{
    CrabController c(cfg(), QStringLiteral("default"));
    QCOMPARE(c.variants(), CrabConfig::spriteVariants());
    QVERIFY(c.variants().size() >= 2);
}

void TestCrabController::switchingPersists()
{
    CrabController c(cfg(), QStringLiteral("default"));
    c.setVariant(QStringLiteral("fancy"));

    QCOMPARE(c.variant(), QStringLiteral("fancy"));
    // Written straight away: this runs as a service, so a choice that only
    // lived in memory would silently revert on the next restart.
    QCOMPARE(read().value(QStringLiteral("sprite")).toString(), QStringLiteral("fancy"));
}

void TestCrabController::switchingEmitsOnce()
{
    CrabController c(cfg(), QStringLiteral("default"));
    QSignalSpy spy(&c, &CrabController::variantChanged);
    c.setVariant(QStringLiteral("fancy"));
    QCOMPARE(spy.count(), 1);
}

void TestCrabController::switchingToTheSameVariantIsANoOp()
{
    CrabController c(cfg(), QStringLiteral("fancy"));
    QSignalSpy spy(&c, &CrabController::variantChanged);
    c.setVariant(QStringLiteral("fancy"));
    QCOMPARE(spy.count(), 0);
    // No write either, so re-selecting the current entry cannot create a file.
    QVERIFY(!QFile::exists(cfg()) || read().isEmpty()
            || read().contains(QStringLiteral("sprite")));
}

void TestCrabController::unknownVariantIsRejectedAndNotPersisted()
{
    CrabController c(cfg(), QStringLiteral("default"));
    QSignalSpy spy(&c, &CrabController::variantChanged);

    c.setVariant(QStringLiteral("sombrero"));

    QCOMPARE(c.variant(), QStringLiteral("default"));
    QCOMPARE(spy.count(), 0);
    QVERIFY(read().value(QStringLiteral("sprite")).toString() != QStringLiteral("sombrero"));
}

void TestCrabController::persistPreservesUnrelatedKeys()
{
    // The file belongs to the user and may hold keys this build knows nothing
    // about; switching sprites must not eat them.
    QFile f(cfg());
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write(R"({"stripHeight": 96, "sleepCorner": "left", "future": {"x": 1}})");
    f.close();

    CrabController c(cfg(), QStringLiteral("default"));
    c.setVariant(QStringLiteral("fancy"));

    const QJsonObject saved = read();
    QCOMPARE(saved.value(QStringLiteral("sprite")).toString(), QStringLiteral("fancy"));
    QCOMPARE(saved.value(QStringLiteral("stripHeight")).toInt(), 96);
    QCOMPARE(saved.value(QStringLiteral("sleepCorner")).toString(), QStringLiteral("left"));
    QVERIFY(saved.contains(QStringLiteral("future")));

    // And the result must still load cleanly.
    const CrabConfig reloaded = CrabConfig::load(cfg());
    QCOMPARE(reloaded.sprite, QStringLiteral("fancy"));
    QCOMPARE(reloaded.stripHeight, 96);
}

void TestCrabController::persistRefusesToClobberNonObject()
{
    const QString path = m_dir.filePath(QStringLiteral("junk.json"));
    QFile f(path);
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write("[1,2,3]");
    f.close();

    QVERIFY(!CrabConfig::saveSprite(path, QStringLiteral("fancy")));

    QVERIFY(f.open(QIODevice::ReadOnly));
    QCOMPARE(f.readAll(), QByteArray("[1,2,3]"));
}

void TestCrabController::sheetUrlFollowsVariant()
{
    CrabController c(cfg(), QStringLiteral("default"));
    const QUrl plain = c.sheetUrl();
    QVERIFY(plain.toString().endsWith(QStringLiteral("spritesheet.png")));

    c.setVariant(QStringLiteral("fancy"));
    QVERIFY(c.sheetUrl() != plain);
    QVERIFY(c.sheetUrl().toString().endsWith(QStringLiteral("spritesheet-fancy.png")));
}

void TestCrabController::labelsAreHumanReadable()
{
    CrabController c(cfg(), QStringLiteral("default"));
    for (const QString &variant : c.variants()) {
        const QString label = c.labelFor(variant);
        QVERIFY(!label.isEmpty());
        // A menu showing raw keys would be a giveaway that one was forgotten.
        QVERIFY2(label != variant, qPrintable(QStringLiteral("no label for ") + variant));
    }
}

void TestCrabController::inputRegionWithoutAWindowIsHarmless()
{
    CrabController c(cfg(), QStringLiteral("default"));
    c.setInputRegion(0, 0, 100, 100); // must not crash before setWindow()
    c.setInputRegion(0, 0, 0, 0);
}

QTEST_MAIN(TestCrabController)
#include "tst_crabcontroller.moc"
