/*
 * SPDX-License-Identifier: MIT
 */

#include "CrabConfig.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QSaveFile>
#include <QJsonObject>
#include <QLoggingCategory>
#include <QStandardPaths>

Q_DECLARE_LOGGING_CATEGORY(CRAB)

QStringList CrabConfig::spriteVariants()
{
    // Mirrors VARIANTS in tools/gen_sprites.py; adding one there means adding
    // it here and to the CMake resource list.
    return {QStringLiteral("default"), QStringLiteral("fancy")};
}

QString CrabConfig::spriteFileName() const
{
    return sprite == QLatin1String("fancy") ? QStringLiteral("spritesheet-fancy.png")
                                            : QStringLiteral("spritesheet.png");
}

bool CrabConfig::insideFlatpak()
{
    return QFile::exists(QStringLiteral("/.flatpak-info"));
}

QString CrabConfig::inboxDir(bool flatpak)
{
    // An explicit override always wins: it is the escape hatch for a host with
    // a non-default XDG_STATE_HOME, which a sandbox cannot otherwise discover.
    const QByteArray override = qgetenv("CLAUDE_CRAB_STATE_DIR");
    if (!override.isEmpty()) {
        return QString::fromLocal8Bit(override) + QStringLiteral("/inbox");
    }

    QString base;
    if (!flatpak) {
        base = QString::fromLocal8Bit(qgetenv("XDG_STATE_HOME"));
    }
    // Inside a Flatpak, XDG_STATE_HOME describes the sandbox's own state, not
    // the host's, so it is skipped in favour of the conventional location --
    // which the manifest binds through at the same path.
    if (base.isEmpty()) {
        base = QDir::homePath() + QStringLiteral("/.local/state");
    }
    return base + QStringLiteral("/claude-crab/inbox");
}

QString CrabConfig::defaultPath()
{
    return QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation)
        + QStringLiteral("/claude-crab.json");
}

CrabConfig CrabConfig::load(const QString &path)
{
    CrabConfig config;

    QFile file(path);
    if (!file.exists()) {
        return config;
    }
    if (!file.open(QIODevice::ReadOnly)) {
        qCWarning(CRAB) << "cannot read" << path << "- using defaults";
        return config;
    }

    QJsonParseError error;
    const QJsonDocument doc = QJsonDocument::fromJson(file.readAll(), &error);
    if (error.error != QJsonParseError::NoError || !doc.isObject()) {
        qCWarning(CRAB) << path << "is not a JSON object -" << error.errorString()
                        << "- using defaults";
        return config;
    }

    const QJsonObject obj = doc.object();
    config.stripHeight = obj.value(QLatin1String("stripHeight")).toInt(config.stripHeight);
    config.menuHeadroom = obj.value(QLatin1String("menuHeadroom")).toInt(config.menuHeadroom);
    config.crabScale = obj.value(QLatin1String("crabScale")).toDouble(config.crabScale);
    config.output = obj.value(QLatin1String("output")).toString(config.output);
    config.sleepCorner = obj.value(QLatin1String("sleepCorner")).toString(config.sleepCorner);
    config.sprite = obj.value(QLatin1String("sprite")).toString(config.sprite);
    config.staleTimeoutMinutes =
        obj.value(QLatin1String("staleTimeoutMinutes")).toInt(config.staleTimeoutMinutes);
    config.inboxMaxAgeMinutes =
        obj.value(QLatin1String("inboxMaxAgeMinutes")).toInt(config.inboxMaxAgeMinutes);
    config.inboxMaxMegabytes =
        obj.value(QLatin1String("inboxMaxMegabytes")).toInt(config.inboxMaxMegabytes);

    if (obj.value(QLatin1String("reactions")).isObject()) {
        const QJsonObject reactions = obj.value(QLatin1String("reactions")).toObject();
        for (auto it = reactions.constBegin(); it != reactions.constEnd(); ++it) {
            if (config.reactions.contains(it.key())) {
                config.reactions[it.key()] = it.value().toBool(true);
            } else {
                qCWarning(CRAB) << "unknown reaction" << it.key() << "- ignoring";
            }
        }
    }

    if (config.sleepCorner != QLatin1String("left")
        && config.sleepCorner != QLatin1String("right")) {
        qCWarning(CRAB) << "sleepCorner must be 'left' or 'right', got" << config.sleepCorner;
        config.sleepCorner = QStringLiteral("right");
    }
    if (!CrabConfig::spriteVariants().contains(config.sprite)) {
        qCWarning(CRAB) << "unknown sprite variant" << config.sprite << "- using default;"
                        << "known variants:" << CrabConfig::spriteVariants();
        config.sprite = QStringLiteral("default");
    }
    if (config.menuHeadroom < 0) {
        qCWarning(CRAB) << "menuHeadroom cannot be negative; using 0";
        config.menuHeadroom = 0;
    }
    // A zero or negative budget would disable pruning entirely, which is the
    // one setting that cannot be allowed: the inbox would grow without bound.
    if (config.inboxMaxAgeMinutes < 1) {
        qCWarning(CRAB) << "inboxMaxAgeMinutes must be at least 1; using 60";
        config.inboxMaxAgeMinutes = 60;
    }
    if (config.inboxMaxMegabytes < 1) {
        qCWarning(CRAB) << "inboxMaxMegabytes must be at least 1; using 32";
        config.inboxMaxMegabytes = 32;
    }
    if (config.stripHeight < 16) {
        qCWarning(CRAB) << "stripHeight" << config.stripHeight << "is too small; using 72";
        config.stripHeight = 72;
    }

    return config;
}

bool CrabConfig::saveSprite(const QString &path, const QString &variant)
{
    // Read-modify-write rather than serialising the whole struct: the file
    // belongs to the user, who may have keys this build does not know about.
    QJsonObject obj;
    QFile in(path);
    if (in.exists() && in.open(QIODevice::ReadOnly)) {
        const QJsonDocument doc = QJsonDocument::fromJson(in.readAll());
        if (doc.isObject()) {
            obj = doc.object();
        } else if (in.size() > 0) {
            qCWarning(CRAB) << path << "is not a JSON object; refusing to overwrite it";
            return false;
        }
        in.close();
    }

    obj.insert(QLatin1String("sprite"), variant);

    QDir().mkpath(QFileInfo(path).absolutePath());
    QSaveFile out(path);
    if (!out.open(QIODevice::WriteOnly)) {
        qCWarning(CRAB) << "cannot write" << path << out.errorString();
        return false;
    }
    out.write(QJsonDocument(obj).toJson(QJsonDocument::Indented));
    return out.commit();
}

QVariantMap CrabConfig::toVariantMap() const
{
    return {
        {QStringLiteral("stripHeight"), stripHeight},
        {QStringLiteral("menuHeadroom"), menuHeadroom},
        {QStringLiteral("crabScale"), crabScale},
        {QStringLiteral("output"), output},
        {QStringLiteral("sleepCorner"), sleepCorner},
        {QStringLiteral("sprite"), sprite},
        {QStringLiteral("staleTimeoutMinutes"), staleTimeoutMinutes},
        {QStringLiteral("inboxMaxAgeMinutes"), inboxMaxAgeMinutes},
        {QStringLiteral("inboxMaxMegabytes"), inboxMaxMegabytes},
        {QStringLiteral("reactions"), reactions},
    };
}
