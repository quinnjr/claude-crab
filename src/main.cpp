/*
 * SPDX-License-Identifier: MIT
 */

#include "CrabConfig.h"
#include "SessionTracker.h"

#include <QCommandLineParser>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLoggingCategory>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickWindow>
#include <QScreen>
#include <QStandardPaths>
#include <QTimer>

#include <LayerShellQt/Window>

Q_DECLARE_LOGGING_CATEGORY(CRAB)

namespace
{

QString inboxDir()
{
    return QStandardPaths::writableLocation(QStandardPaths::GenericStateLocation)
        + QStringLiteral("/claude-crab/inbox");
}

/**
 * Load the generated manifest out of the binary's resources.
 *
 * This deliberately happens in C++ rather than via XMLHttpRequest in QML: the
 * manifest is a build artefact compiled into the executable, so a failure here
 * is a packaging bug that must be reported loudly, not swallowed by an async
 * callback that may never fire.
 */
QVariantMap loadManifest()
{
    QFile file(QStringLiteral(":/qt/qml/ClaudeCrab/assets/manifest.json"));
    if (!file.open(QIODevice::ReadOnly)) {
        qCCritical(CRAB) << "sprite manifest missing from resources; the build is broken";
        return {};
    }

    QJsonParseError error;
    const QJsonDocument doc = QJsonDocument::fromJson(file.readAll(), &error);
    if (error.error != QJsonParseError::NoError || !doc.isObject()) {
        qCCritical(CRAB) << "sprite manifest is not valid JSON:" << error.errorString();
        return {};
    }
    const QVariantMap manifest = doc.object().toVariantMap();
    qCInfo(CRAB) << "sprite manifest:" << manifest.value(QStringLiteral("animations")).toList().size()
                 << "animations," << manifest.value(QStringLiteral("frameWidth")).toInt() << "px frames";
    return manifest;
}

QScreen *pickScreen(const QString &connector)
{
    if (connector.isEmpty()) {
        return QGuiApplication::primaryScreen();
    }
    const auto screens = QGuiApplication::screens();
    for (QScreen *screen : screens) {
        if (screen->name() == connector) {
            return screen;
        }
    }
    qCWarning(CRAB) << "output" << connector << "not found; falling back to primary";
    return QGuiApplication::primaryScreen();
}

/**
 * Feeds a recorded JSONL event log into the tracker on a timer, so every
 * animation can be exercised deterministically without running Claude Code.
 */
void startReplay(SessionTracker *tracker, const QString &path, int intervalMs)
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        qCWarning(CRAB) << "cannot open replay file" << path;
        QCoreApplication::exit(1);
        return;
    }

    auto lines = std::make_shared<QList<QByteArray>>();
    while (!file.atEnd()) {
        const QByteArray line = file.readLine().trimmed();
        if (!line.isEmpty()) {
            lines->append(line);
        }
    }
    if (lines->isEmpty()) {
        qCWarning(CRAB) << "replay file" << path << "is empty";
        return;
    }

    auto index = std::make_shared<int>(0);
    auto *timer = new QTimer(tracker);
    timer->setInterval(intervalMs);
    QObject::connect(timer, &QTimer::timeout, tracker, [tracker, lines, index] {
        const QJsonDocument doc = QJsonDocument::fromJson(lines->at(*index));
        if (doc.isObject()) {
            tracker->handleEvent(doc.object(), QDateTime::currentMSecsSinceEpoch());
        }
        *index = (*index + 1) % lines->size(); // loop, so it runs until quit
    });
    timer->start();
}

} // namespace

int main(int argc, char *argv[])
{
    // LayerShellQt::Shell::useLayerShell() is deprecated since Qt 6.5 -- the
    // platform plugin promotes a window to a layer surface as soon as
    // LayerShellQt::Window::get() is called on it, before it is shown.
    QGuiApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("claude-crab"));
    app.setApplicationVersion(QStringLiteral("1.0.0"));
    app.setOrganizationDomain(QStringLiteral("quinnjr.dev"));
    // Reverse-DNS app id, matching the convention used by this author's
    // other Plasma work. KWin keys window rules and stacking off it.
    app.setDesktopFileName(QStringLiteral("dev.quinnjr.claude-crab"));

    QCommandLineParser parser;
    parser.setApplicationDescription(
        QStringLiteral("A crab that walks above your panel while Claude Code works."));
    parser.addHelpOption();
    parser.addVersionOption();

    QCommandLineOption demoOption(QStringLiteral("demo"),
                                  QStringLiteral("Cycle every animation on a timer."));
    QCommandLineOption replayOption(QStringLiteral("replay"),
                                    QStringLiteral("Replay a recorded JSONL event log."),
                                    QStringLiteral("file"));
    QCommandLineOption configOption(QStringLiteral("config"),
                                    QStringLiteral("Path to claude-crab.json."),
                                    QStringLiteral("file"), CrabConfig::defaultPath());
    QCommandLineOption spriteOption(
        QStringLiteral("sprite"),
        QStringLiteral("Sprite variant, overriding the config file: %1.")
            .arg(CrabConfig::spriteVariants().join(QStringLiteral(", "))),
        QStringLiteral("variant"));
    parser.addOption(demoOption);
    parser.addOption(replayOption);
    parser.addOption(configOption);
    parser.addOption(spriteOption);
    parser.process(app);

    CrabConfig config = CrabConfig::load(parser.value(configOption));
    if (parser.isSet(spriteOption)) {
        const QString requested = parser.value(spriteOption);
        if (!CrabConfig::spriteVariants().contains(requested)) {
            qCCritical(CRAB) << "unknown sprite variant" << requested << "- expected one of"
                             << CrabConfig::spriteVariants();
            return 2;
        }
        config.sprite = requested;
    }
    qCInfo(CRAB) << "sprite variant:" << config.sprite;

    auto *tracker = new SessionTracker(inboxDir(), &app);
    tracker->setStaleTimeoutMs(qint64(config.staleTimeoutMinutes) * 60 * 1000);

    const bool demo = parser.isSet(demoOption);
    const bool replay = parser.isSet(replayOption);

    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty(QStringLiteral("tracker"), tracker);
    engine.rootContext()->setContextProperty(QStringLiteral("crabConfig"),
                                             config.toVariantMap());
    engine.rootContext()->setContextProperty(QStringLiteral("demoMode"), demo);
    engine.rootContext()->setContextProperty(QStringLiteral("crabManifest"), loadManifest());
    engine.rootContext()->setContextProperty(
        QStringLiteral("spriteSheetUrl"),
        QStringLiteral("qrc:/qt/qml/ClaudeCrab/assets/") + config.spriteFileName());

    engine.loadFromModule(QStringLiteral("ClaudeCrab"), QStringLiteral("Main"));
    if (engine.rootObjects().isEmpty()) {
        qCCritical(CRAB) << "QML failed to load";
        return 1;
    }

    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().constFirst());
    if (!window) {
        qCCritical(CRAB) << "root QML object is not a Window";
        return 1;
    }

    window->setColor(Qt::transparent);
    window->setFlag(Qt::WindowTransparentForInput); // clicks pass through
    window->setHeight(config.stripHeight);

    QScreen *screen = pickScreen(config.output);
    if (screen) {
        window->setScreen(screen);
    }

    if (QGuiApplication::platformName().contains(QLatin1String("wayland"))) {
        if (auto *layer = LayerShellQt::Window::get(window)) {
            layer->setScope(QStringLiteral("dev.quinnjr.claude-crab"));
            layer->setLayer(LayerShellQt::Window::LayerTop);
            // Braced init: LayerShellQt declares the flags but not the bitwise
            // operators, so `a | b` would decay to int.
            layer->setAnchors({LayerShellQt::Window::AnchorBottom,
                               LayerShellQt::Window::AnchorLeft,
                               LayerShellQt::Window::AnchorRight});
            // Reserve nothing ourselves, but respect the panel's reservation so
            // we land directly above it rather than behind it.
            layer->setExclusiveZone(0);
            layer->setKeyboardInteractivity(
                LayerShellQt::Window::KeyboardInteractivityNone);
            layer->setActivateOnShow(false);
            layer->setDesiredSize(QSize(0, config.stripHeight));
            if (screen) {
                layer->setScreen(screen);
            }
        } else {
            qCWarning(CRAB) << "layer-shell unavailable; window will be a plain surface";
        }
    } else {
        // X11 or offscreen: no layer-shell. Degrade to an always-on-top frameless
        // window positioned by screen geometry.
        qCWarning(CRAB) << "not a Wayland session; using the X11 fallback window";
        window->setFlags(window->flags() | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint
                         | Qt::Tool);
        if (screen) {
            const QRect available = screen->availableGeometry();
            window->setGeometry(available.x(), available.bottom() - config.stripHeight + 1,
                                available.width(), config.stripHeight);
        }
    }

    if (replay) {
        startReplay(tracker, parser.value(replayOption), 900);
    } else if (!demo) {
        tracker->start();
    }

    window->show();
    return app.exec();
}
