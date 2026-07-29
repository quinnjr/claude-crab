/*
 * SPDX-License-Identifier: MIT
 */

#pragma once

#include <QString>
#include <QVariantMap>

/**
 * Reads ~/.config/claude-crab.json. Every key is optional; a missing or broken
 * file yields defaults rather than an error, because a config typo should not
 * cost you the crab.
 */
struct CrabConfig {
    int stripHeight = 72;
    qreal crabScale = 1.0;
    QString output; // connector name, e.g. "DP-1"; empty means primary
    QString sleepCorner = QStringLiteral("right");
    int staleTimeoutMinutes = 10;
    QVariantMap reactions{
        {QStringLiteral("waiting"), true},
        {QStringLiteral("finished"), true},
        {QStringLiteral("error"), true},
        {QStringLiteral("toolFlavour"), true},
    };

    static QString defaultPath();
    static CrabConfig load(const QString &path);

    /** Shape handed to QML. */
    QVariantMap toVariantMap() const;
};
