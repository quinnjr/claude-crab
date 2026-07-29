/*
 * SPDX-License-Identifier: MIT
 */

#include "CrabController.h"

#include "CrabConfig.h"

#include <QLoggingCategory>
#include <QQuickWindow>
#include <QRegion>

Q_DECLARE_LOGGING_CATEGORY(CRAB)

CrabController::CrabController(QString configPath, QString variant, QObject *parent)
    : QObject(parent)
    , m_configPath(std::move(configPath))
    , m_variant(std::move(variant))
{
}

QStringList CrabController::variants() const
{
    return CrabConfig::spriteVariants();
}

QUrl CrabController::sheetUrl() const
{
    CrabConfig config;
    config.sprite = m_variant;
    return QUrl(QStringLiteral("qrc:/qt/qml/ClaudeCrab/assets/") + config.spriteFileName());
}

QString CrabController::labelFor(const QString &variant) const
{
    if (variant == QLatin1String("fancy")) {
        return tr("Top Hat and Monocle");
    }
    if (variant == QLatin1String("party")) {
        return tr("Party Hat");
    }
    if (variant == QLatin1String("default")) {
        return tr("Plain");
    }
    return variant;
}

void CrabController::setVariant(const QString &variant)
{
    if (variant == m_variant) {
        return;
    }
    if (!CrabConfig::spriteVariants().contains(variant)) {
        qCWarning(CRAB) << "refusing to switch to unknown sprite variant" << variant;
        return;
    }

    m_variant = variant;
    Q_EMIT variantChanged();

    // Persisted immediately: this runs as a systemd service, so a choice that
    // vanished on the next restart would read as the switch not working.
    if (!CrabConfig::saveSprite(m_configPath, m_variant)) {
        qCWarning(CRAB) << "sprite switched to" << m_variant << "but could not be saved to"
                        << m_configPath << "- it will revert on restart";
    }
}

void CrabController::setWindow(QQuickWindow *window)
{
    m_window = window;
    if (m_window) {
        // Nothing is interactive until QML says otherwise.
        m_window->setMask(QRegion());
    }
}

void CrabController::setInputRegion(int x, int y, int width, int height)
{
    if (!m_window) {
        return;
    }

    const QRect rect(x, y, width, height);
    if (rect.isEmpty()) {
        if (!m_region.isEmpty()) {
            m_region = QRect();
            m_window->setMask(QRegion());
        }
        return;
    }

    // Size changes always apply; position changes only past a threshold, so a
    // walking character does not commit a new input region every frame.
    const bool resized = rect.size() != m_region.size();
    const bool moved = (rect.topLeft() - m_region.topLeft()).manhattanLength()
        >= RegionMoveThreshold;
    if (!resized && !moved) {
        return;
    }

    m_region = rect;
    m_window->setMask(QRegion(rect));
    qCDebug(CRAB) << "input region ->" << rect;
}
