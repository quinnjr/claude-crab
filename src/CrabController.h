/*
 * SPDX-License-Identifier: MIT
 */

#pragma once

#include <QObject>
#include <QRect>
#include <QString>
#include <QStringList>
#include <QUrl>

class QQuickWindow;

/**
 * Runtime control surface exposed to QML: which sprite variant is showing, and
 * which part of the window accepts input.
 *
 * The window is otherwise input-transparent. Rather than a global click-through
 * flag, the input region is set to the character's own rectangle so a right
 * click on the character opens its menu while every other pixel of the strip
 * still passes clicks to whatever is underneath.
 */
class CrabController : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString variant READ variant WRITE setVariant NOTIFY variantChanged)
    Q_PROPERTY(QStringList variants READ variants CONSTANT)
    Q_PROPERTY(QUrl sheetUrl READ sheetUrl NOTIFY variantChanged)

public:
    CrabController(QString configPath, QString variant, QObject *parent = nullptr);

    QString variant() const
    {
        return m_variant;
    }
    QStringList variants() const;
    QUrl sheetUrl() const;

    /** Human-readable label for a variant, for menu entries. */
    Q_INVOKABLE QString labelFor(const QString &variant) const;

    /**
     * Set the variant and persist it, so the choice survives the restart that
     * a systemd-managed service makes routine.
     */
    void setVariant(const QString &variant);

    void setWindow(QQuickWindow *window);

    /**
     * Restrict input to @p rect, in window coordinates. An empty rect makes the
     * whole window click-through.
     *
     * Called as the character walks, so it coalesces sub-threshold moves: each
     * change is a wl_surface.set_input_region plus a commit, and issuing one
     * per frame is wasteful for a rectangle that has shifted by a pixel.
     */
    Q_INVOKABLE void setInputRegion(int x, int y, int width, int height);

Q_SIGNALS:
    void variantChanged();

private:
    static constexpr int RegionMoveThreshold = 6;

    QString m_configPath;
    QString m_variant;
    QQuickWindow *m_window = nullptr;
    QRect m_region;
};
