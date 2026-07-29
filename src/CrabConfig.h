/*
 * SPDX-License-Identifier: MIT
 */

#pragma once

#include <QString>
#include <QStringList>
#include <QVariantMap>

/**
 * Reads ~/.config/claude-crab.json. Every key is optional; a missing or broken
 * file yields defaults rather than an error, because a config typo should not
 * cost you the crab.
 */
struct CrabConfig {
    int stripHeight = 72;
    /** Transparent space reserved above the strip for the right-click menu. */
    int menuHeadroom = 220;
    qreal crabScale = 1.0;
    QString output; // connector name, e.g. "DP-1"; empty means primary
    QString sleepCorner = QStringLiteral("right");
    /** Sprite variant: "default" or "fancy" (top hat and monocle). */
    QString sprite = QStringLiteral("default");
    int staleTimeoutMinutes = 10;
    /** Inbox budget. Hooks keep writing while the crab is stopped, so the
     *  directory needs an upper bound in both age and size. */
    int inboxMaxAgeMinutes = 60;
    int inboxMaxMegabytes = 32;
    QVariantMap reactions{
        {QStringLiteral("waiting"), true},
        {QStringLiteral("finished"), true},
        {QStringLiteral("error"), true},
        {QStringLiteral("toolFlavour"), true},
    };

    static QString defaultPath();
    /** Sprite variant names this build knows how to render. */
    static QStringList spriteVariants();
    /** Sheet file name for the selected variant. */
    QString spriteFileName() const;
    static CrabConfig load(const QString &path);
    /**
     * Rewrite just the "sprite" key at @p path, preserving every other key and
     * any formatting-independent content. Returns false on failure.
     */
    static bool saveSprite(const QString &path, const QString &variant);

    /** Shape handed to QML. */
    QVariantMap toVariantMap() const;
};
