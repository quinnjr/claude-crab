/*
 * SPDX-License-Identifier: MIT
 */

#include "CrabConfig.h"

#include <QFile>
#include <QJsonDocument>
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
    config.crabScale = obj.value(QLatin1String("crabScale")).toDouble(config.crabScale);
    config.output = obj.value(QLatin1String("output")).toString(config.output);
    config.sleepCorner = obj.value(QLatin1String("sleepCorner")).toString(config.sleepCorner);
    config.sprite = obj.value(QLatin1String("sprite")).toString(config.sprite);
    config.staleTimeoutMinutes =
        obj.value(QLatin1String("staleTimeoutMinutes")).toInt(config.staleTimeoutMinutes);

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
    if (config.stripHeight < 16) {
        qCWarning(CRAB) << "stripHeight" << config.stripHeight << "is too small; using 72";
        config.stripHeight = 72;
    }

    return config;
}

QVariantMap CrabConfig::toVariantMap() const
{
    return {
        {QStringLiteral("stripHeight"), stripHeight},
        {QStringLiteral("crabScale"), crabScale},
        {QStringLiteral("output"), output},
        {QStringLiteral("sleepCorner"), sleepCorner},
        {QStringLiteral("sprite"), sprite},
        {QStringLiteral("staleTimeoutMinutes"), staleTimeoutMinutes},
        {QStringLiteral("reactions"), reactions},
    };
}
