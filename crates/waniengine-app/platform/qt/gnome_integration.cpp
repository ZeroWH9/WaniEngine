#include "gnome_integration.h"
#include "runtime_paths.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QProcessEnvironment>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQuickItem>
#include <QQuickView>
#include <QSaveFile>
#include <QScreen>
#include <QSocketNotifier>
#include <QStandardPaths>
#include <QTextStream>
#include <QUrl>
#include <QWindow>
#include <algorithm>
#include <fcntl.h>
#ifdef WANIENGINE_HAS_X11
#include <X11/Xlib.h>
#include <X11/Xatom.h>
#endif
#include <unistd.h>

using namespace Qt::StringLiterals;

namespace {
bool localMedia(const QString &url)
{
    const QUrl parsed(url);
    return parsed.isLocalFile() && QFileInfo(parsed.toLocalFile()).isFile();
}

QString queryGSettings(const QString &schema, const QString &key)
{
    QProcess process;
    process.start(u"gsettings"_s, {u"get"_s, schema, key});
    if (process.waitForFinished(2000) && process.exitCode() == 0) {
        return QString::fromUtf8(process.readAllStandardOutput()).trimmed();
    }
    return {};
}

void setGSettings(const QString &schema, const QString &key, const QString &value)
{
    if (value.isEmpty()) return;
    QProcess process;
    process.start(u"gsettings"_s, {u"set"_s, schema, key, value});
    process.waitForFinished(2000);
}

#ifdef WANIENGINE_HAS_X11
void configureX11Wallpaper(QWindow *window)
{
    auto *display = XOpenDisplay(nullptr);
    if (!display) return;

    const Window nativeWindow = static_cast<Window>(window->winId());
    const Atom windowType = XInternAtom(display, "_NET_WM_WINDOW_TYPE", False);
    const Atom desktopType = XInternAtom(display, "_NET_WM_WINDOW_TYPE_DESKTOP", False);
    XChangeProperty(display, nativeWindow, windowType, XA_ATOM, 32, PropModeReplace,
                    reinterpret_cast<const unsigned char *>(&desktopType), 1);

    const Atom state = XInternAtom(display, "_NET_WM_STATE", False);
    const Atom states[] = {
        XInternAtom(display, "_NET_WM_STATE_BELOW", False),
        XInternAtom(display, "_NET_WM_STATE_SKIP_TASKBAR", False),
        XInternAtom(display, "_NET_WM_STATE_SKIP_PAGER", False),
        XInternAtom(display, "_NET_WM_STATE_STICKY", False),
    };
    XChangeProperty(display, nativeWindow, state, XA_ATOM, 32, PropModeReplace,
                    reinterpret_cast<const unsigned char *>(states),
                    static_cast<int>(std::size(states)));

    const Atom desktop = XInternAtom(display, "_NET_WM_DESKTOP", False);
    const unsigned long allDesktops = 0xFFFFFFFFUL;
    XChangeProperty(display, nativeWindow, desktop, XA_CARDINAL, 32, PropModeReplace,
                    reinterpret_cast<const unsigned char *>(&allDesktops), 1);
    XLowerWindow(display, nativeWindow);
    XFlush(display);
    XCloseDisplay(display);
}
#endif
}

GnomeIntegration::GnomeIntegration(QObject *parent)
    : QObject(parent),
      m_sessionPath((qEnvironmentVariableIsEmpty("XDG_STATE_HOME") ? QDir::homePath() + u"/.local/state"_s : qEnvironmentVariable("XDG_STATE_HOME")) + u"/WaniEngine/gnome-session.json"_s)
{
    if (auto *gui = qobject_cast<QGuiApplication *>(QCoreApplication::instance())) {
        auto trackScreen = [this](QScreen *screen) {
            connect(screen, &QScreen::refreshRateChanged, this, &GnomeIntegration::monitorsChanged);
            connect(screen, &QScreen::geometryChanged, this, &GnomeIntegration::monitorsChanged);
        };
        for (auto *screen : gui->screens()) trackScreen(screen);
        connect(gui, &QGuiApplication::screenAdded, this, [this, trackScreen](QScreen *screen) {
            trackScreen(screen);
            emit monitorsChanged();
        });
        connect(gui, &QGuiApplication::screenRemoved, this, &GnomeIntegration::monitorsChanged);
    }
    loadRecovery();
}

GnomeIntegration::~GnomeIntegration()
{
    restore();
}

bool GnomeIntegration::isActive() const
{
    return m_active;
}

QString GnomeIntegration::lastError() const
{
    return m_lastError;
}

QVariantList GnomeIntegration::monitors() const
{
    QVariantList result;
    if (!qobject_cast<QGuiApplication *>(QCoreApplication::instance())) return result;
    for (QScreen *screen : QGuiApplication::screens()) {
        result.push_back(QVariantMap{
            {u"name"_s, screen->name()},
            {u"width"_s, screen->size().width()},
            {u"height"_s, screen->size().height()},
            {u"pixelWidth"_s, qRound(screen->size().width() * screen->devicePixelRatio())},
            {u"pixelHeight"_s, qRound(screen->size().height() * screen->devicePixelRatio())},
            {u"refreshRate"_s, screen->refreshRate()},
            {u"scale"_s, screen->devicePixelRatio()}
        });
    }
    return result;
}

void GnomeIntegration::setError(const QString &message)
{
    if (m_lastError == message) return;
    m_lastError = message;
    emit lastErrorChanged();
}

void GnomeIntegration::backupStaticWallpaper()
{
    if (!m_originalPictureUri.isEmpty()) return;
    m_originalPictureUri = queryGSettings(u"org.gnome.desktop.background"_s, u"picture-uri"_s);
    m_originalPictureUriDark = queryGSettings(u"org.gnome.desktop.background"_s, u"picture-uri-dark"_s);
    if (m_originalPictureUri.isEmpty()) return;
    // Remember what we wrote over so restore() only rewinds our own change.
    m_appliedPictureUri = m_originalPictureUri;
    m_appliedPictureUriDark = m_originalPictureUriDark;
}

void GnomeIntegration::restoreStaticWallpaper()
{
    const QString current = queryGSettings(u"org.gnome.desktop.background"_s, u"picture-uri"_s);
    const QString currentDark = queryGSettings(u"org.gnome.desktop.background"_s, u"picture-uri-dark"_s);
    const bool pictureUntouched = current == m_appliedPictureUri || current.isEmpty();
    const bool darkUntouched = currentDark == m_appliedPictureUriDark || currentDark.isEmpty();

    if (!m_originalPictureUri.isEmpty() && pictureUntouched) {
        setGSettings(u"org.gnome.desktop.background"_s, u"picture-uri"_s, m_originalPictureUri);
    }
    if (!m_originalPictureUriDark.isEmpty() && darkUntouched) {
        setGSettings(u"org.gnome.desktop.background"_s, u"picture-uri-dark"_s, m_originalPictureUriDark);
    }
    m_originalPictureUri.clear();
    m_originalPictureUriDark.clear();
    m_appliedPictureUri.clear();
    m_appliedPictureUriDark.clear();
}

bool GnomeIntegration::applyContent(const QString &selectedUrl, const QString &selectedKind,
                                    const QString &actorsJson, const QString &monitorName,
                                    bool audioEnabled, const QString &fitMode,
                                    int speed, int volume,
                                    bool playing, bool loop,
                                    const QString &visualOptions, const QString &audioOptions)
{
    Q_UNUSED(actorsJson);
    if (!localMedia(selectedUrl) || (selectedKind != u"wallpaper"_s && selectedKind != u"character"_s)) {
        setError(u"Select a valid local image, animation or video."_s);
        return false;
    }

    backupStaticWallpaper();

    QScreen *targetScreen = nullptr;
    for (auto *screen : QGuiApplication::screens()) {
        if (screen->name() == monitorName) {
            targetScreen = screen;
            break;
        }
    }
    if (!targetScreen && !QGuiApplication::screens().isEmpty()) {
        targetScreen = QGuiApplication::screens().first();
    }
    if (!targetScreen) {
        setError(u"Monitor not found."_s);
        return false;
    }

    const QString actualMonitor = targetScreen->name();
    auto &workerState = m_workers[actualMonitor];

    if (!workerState.process || workerState.process->state() == QProcess::NotRunning) {
        delete workerState.process;
        workerState.process = new QProcess(this);

        auto environment = QProcessEnvironment::systemEnvironment();
        // Prefer XWayland so the window can be layered with
        // _NET_WM_WINDOW_TYPE_DESKTOP. A pure Wayland session keeps its own
        // platform; Mutter then treats the window as a normal bottom layer.
        if (!environment.value(u"DISPLAY"_s).isEmpty()) {
            environment.insert(u"QT_QPA_PLATFORM"_s, u"xcb"_s);
        } else {
            environment.remove(u"QT_QPA_PLATFORM"_s);
        }
        workerState.process->setProcessEnvironment(environment);

        workerState.process->start(QCoreApplication::applicationFilePath(),
                                   {u"--gnome-wallpaper-worker"_s, actualMonitor});

        if (!workerState.process->waitForStarted(3000)) {
            setError(u"Could not launch the GNOME wallpaper window."_s);
            delete workerState.process;
            workerState.process = nullptr;
            return false;
        }

        connect(workerState.process, &QProcess::finished, this, [this, actualMonitor] {
            m_workers.remove(actualMonitor);
            const bool anyActive = !m_workers.isEmpty();
            if (anyActive != m_active) {
                m_active = anyActive;
                emit activeChanged();
            }
        });
    }

    QString kind = u"video"_s;
    const QString suffix = QFileInfo(QUrl(selectedUrl).toLocalFile()).suffix().toLower();
    if (QStringList{u"png"_s, u"jpg"_s, u"jpeg"_s, u"bmp"_s, u"webp"_s}.contains(suffix)) {
        kind = u"image"_s;
    } else if (suffix == u"gif"_s) {
        kind = u"animated"_s;
    }

    QJsonObject config;
    config[u"ContentUrl"_s] = selectedUrl;
    config[u"ContentKind"_s] = kind;
    config[u"FitMode"_s] = fitMode;
    config[u"AudioEnabled"_s] = audioEnabled;
    config[u"Speed"_s] = std::clamp(speed, 10, 300);
    config[u"Volume"_s] = std::clamp(volume, 0, 100);
    config[u"Playing"_s] = playing;
    config[u"Loop"_s] = loop;
    config[u"VisualOptionsJson"_s] = visualOptions;
    config[u"AudioOptionsJson"_s] = audioOptions;

    workerState.parameters = config;
    const QByteArray payload = QJsonDocument(config).toJson(QJsonDocument::Compact) + '\n';
    workerState.process->write(payload);
    workerState.process->waitForBytesWritten(1000);

    m_active = true;
    emit activeChanged();
    saveRecovery();
    setError({});
    return true;
}

bool GnomeIntegration::updateContentOptions(const QString &contentUrl, const QString &monitorName, const QString &json)
{
    Q_UNUSED(contentUrl);
    auto it = m_workers.find(monitorName);
    if (it == m_workers.end() || !it.value().process || it.value().process->state() != QProcess::Running) {
        return false;
    }

    const auto doc = QJsonDocument::fromJson(json.toUtf8());
    if (!doc.isObject()) return false;

    QJsonObject current = it.value().parameters;
    const auto updates = doc.object();
    for (auto i = updates.begin(); i != updates.end(); ++i) {
        current.insert(i.key(), i.value());
    }
    it.value().parameters = current;

    const QByteArray payload = QJsonDocument(current).toJson(QJsonDocument::Compact) + '\n';
    it.value().process->write(payload);
    it.value().process->waitForBytesWritten(1000);
    return true;
}

bool GnomeIntegration::updateAudioOptions(const QString &json)
{
    const auto doc = QJsonDocument::fromJson(json.toUtf8());
    if (!doc.isObject()) return false;

    for (auto it = m_workers.begin(); it != m_workers.end(); ++it) {
        if (it.value().process && it.value().process->state() == QProcess::Running) {
            it.value().parameters[u"AudioOptionsJson"_s] = QString::fromUtf8(doc.toJson(QJsonDocument::Compact));
            const QByteArray payload = QJsonDocument(it.value().parameters).toJson(QJsonDocument::Compact) + '\n';
            it.value().process->write(payload);
            it.value().process->waitForBytesWritten(500);
        }
    }
    return true;
}

bool GnomeIntegration::updateActors(const QString &actorsJson, const QString &monitorName)
{
    Q_UNUSED(actorsJson);
    Q_UNUSED(monitorName);
    // Actor overlays are handled by OverlayController
    return true;
}

bool GnomeIntegration::restore()
{
    for (auto it = m_workers.begin(); it != m_workers.end(); ++it) {
        if (it.value().process) {
            if (it.value().process->state() != QProcess::NotRunning) {
                it.value().process->kill();
                it.value().process->waitForFinished(100);
            }
            delete it.value().process;
            it.value().process = nullptr;
        }
    }
    m_workers.clear();

    restoreStaticWallpaper();
    QFile::remove(m_sessionPath);

    if (m_active) {
        m_active = false;
        emit activeChanged();
    }
    setError({});
    return true;
}

void GnomeIntegration::saveRecovery() const
{
    QJsonObject root;
    root[u"schema"_s] = 1;
    root[u"originalPictureUri"_s] = m_originalPictureUri;
    root[u"originalPictureUriDark"_s] = m_originalPictureUriDark;

    QJsonArray monitorList;
    for (auto it = m_workers.begin(); it != m_workers.end(); ++it) {
        QJsonObject entry;
        entry[u"monitor"_s] = it.key();
        entry[u"parameters"_s] = it.value().parameters;
        monitorList.append(entry);
    }
    root[u"monitors"_s] = monitorList;

    QSaveFile file(m_sessionPath);
    if (file.open(QIODevice::WriteOnly)) {
        file.write(QJsonDocument(root).toJson(QJsonDocument::Indented));
        file.commit();
    }
}

bool GnomeIntegration::loadRecovery()
{
    QFile file(m_sessionPath);
    if (!file.exists() || !file.open(QIODevice::ReadOnly)) return false;

    const auto root = QJsonDocument::fromJson(file.readAll()).object();
    if (root.isEmpty()) return false;

    m_originalPictureUri = root.value(u"originalPictureUri"_s).toString();
    m_originalPictureUriDark = root.value(u"originalPictureUriDark"_s).toString();
    return true;
}

int GnomeIntegration::runWallpaperWorker(int argc, char **argv)
{
    // Prefer XCB through XWayland; fall back to the session's own platform.
    if (!qEnvironmentVariableIsEmpty("DISPLAY")) {
        qputenv("QT_QPA_PLATFORM", "xcb");
    }

    QGuiApplication app(argc, argv);
    app.setApplicationName(u"waniengine-wallpaper"_s);

    if (argc < 3) return 1;
    const QString monitorName = QString::fromLocal8Bit(argv[2]);

    QScreen *screen = nullptr;
    for (auto *candidate : app.screens()) {
        if (candidate->name() == monitorName) screen = candidate;
    }
    if (!screen && !app.screens().isEmpty()) screen = app.screens().first();
    if (!screen) return 2;

    QQuickView view;
    view.engine()->addImportPath(waniEngineRuntimeRoot());
    view.setTitle(u"WaniEngine Wallpaper — %1"_s.arg(monitorName));

    // _NET_WM_WINDOW_TYPE_DESKTOP pegs window to the background layer in GNOME Mutter
    view.setProperty("_q_xcb_wm_window_type", "_NET_WM_WINDOW_TYPE_DESKTOP");
    view.setFlags(Qt::FramelessWindowHint | Qt::WindowStaysOnBottomHint
                  | Qt::WindowDoesNotAcceptFocus | Qt::WindowTransparentForInput);
    view.setColor(Qt::black);
    view.setResizeMode(QQuickView::SizeRootObjectToView);
    view.setScreen(screen);
    view.setGeometry(screen->geometry());

    const QString wallpaperQml = waniEngineWallpaperSource() + u"/contents/ui/WallpaperContent.qml"_s;
    view.setInitialProperties({{u"configuration"_s, QVariantMap{}}});
    view.setSource(QUrl::fromLocalFile(wallpaperQml));

    if (view.status() == QQuickView::Error) {
        qWarning() << "Failed to load WallpaperContent.qml";
        return 3;
    }

    view.show();
#ifdef WANIENGINE_HAS_X11
    if (app.platformName() == u"xcb"_s) configureX11Wallpaper(&view);
#endif

    // Listen on stdin for JSON updates
    fcntl(STDIN_FILENO, F_SETFL, fcntl(STDIN_FILENO, F_GETFL) | O_NONBLOCK);
    QSocketNotifier notifier(STDIN_FILENO, QSocketNotifier::Read);
    QByteArray buffer;

    QObject::connect(&notifier, &QSocketNotifier::activated, &app, [&] {
        char chunk[4096];
        const qint64 bytes = ::read(STDIN_FILENO, chunk, sizeof(chunk));
        if (bytes <= 0) {
            app.quit();
            return;
        }
        buffer.append(chunk, bytes);
        while (buffer.contains('\n')) {
            const int newline = buffer.indexOf('\n');
            const QByteArray line = buffer.left(newline).trimmed();
            buffer.remove(0, newline + 1);

            if (!line.isEmpty()) {
                const auto doc = QJsonDocument::fromJson(line);
                if (doc.isObject() && view.rootObject()) {
                    view.rootObject()->setProperty("configuration", doc.object().toVariantMap());
                }
            }
        }
    });

    QObject::connect(&app, &QGuiApplication::screenRemoved, &app, [&](QScreen *removed) {
        if (removed == screen) app.quit();
    });

    return app.exec();
}
