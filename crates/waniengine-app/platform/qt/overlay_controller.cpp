#include "runtime_paths.h"
#include <QQmlEngine>
#include "overlay_controller.h"
#ifdef WANIENGINE_HAS_LAYER_SHELL_QT
#include <LayerShellQt/Window>
#endif
#include <QGuiApplication>
#include <QProcessEnvironment>
#include <QDebug>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickView>
#include <QScreen>
#include <QSocketNotifier>
#include <QTextStream>
#include <QTimer>
#include <QUrl>
#include <QFileInfo>
#include <unistd.h>
#include <fcntl.h>
using namespace Qt::StringLiterals;
namespace {
constexpr qsizetype maxMessage = 1024 * 1024;
bool validActors(const QJsonArray &actors) {
    if (actors.size() > 16) return false;
    for (const auto &value : actors) {
        if (!value.isObject()) return false;
        const QUrl url(value.toObject().value(u"url"_s).toString());
        QImageReader reader(url.toLocalFile());
        if (!url.isLocalFile() || !QFileInfo(url.toLocalFile()).isFile() || !reader.canRead()) return false;
        const auto size = reader.size();
        // Keep decompressed animation caches bounded to desktop sticker sizes.
        if (size.width() <= 0 || size.height() <= 0 || size.width() > 2048 || size.height() > 2048) return false;
    }
    return true;
}
}
OverlayController::OverlayController(QObject *parent) : QObject(parent) {
    connect(&m_worker,&QProcess::finished,this,[this](int code,QProcess::ExitStatus status){
        if (code != 0 || status != QProcess::NormalExit) error(u"The animation process stopped. Wallpaper playback remains independent."_s);
        emit activeChanged();
    });
    connect(&m_worker,&QProcess::readyReadStandardError,this,[this]{
        const auto output=m_worker.readAllStandardError();
        if (!output.isEmpty()) qWarning().noquote()<<"Overlay:"<<QString::fromUtf8(output).trimmed();
    });
    connect(qGuiApp,&QGuiApplication::screenRemoved,this,[this](QScreen *screen){if(screen->name()==m_monitor)clear();});
}
OverlayController::~OverlayController(){clear();}
bool OverlayController::active() const {return m_worker.state()==QProcess::Running;}
void OverlayController::error(const QString &message){m_error=message;emit lastErrorChanged();}
void OverlayController::clear(){
    if(m_worker.state()!=QProcess::NotRunning){
        m_worker.closeWriteChannel();
        if(!m_worker.waitForFinished(1500)){m_worker.terminate();if(!m_worker.waitForFinished(1000)){m_worker.kill();m_worker.waitForFinished(1000);}}
    }
    m_monitor.clear(); emit activeChanged();
}
bool OverlayController::applyActors(const QString &json,const QString &monitor){
    const auto document=QJsonDocument::fromJson(json.toUtf8());
    if(!document.isArray() || !validActors(document.array())){error(u"Use up to 16 local images or GIFs, at most 2048 × 2048 pixels."_s);return false;}
    if(document.array().isEmpty()){clear();error({});return true;}

    const bool isKde = qEnvironmentVariable("XDG_CURRENT_DESKTOP").contains(u"KDE"_s);
    if(isKde && QGuiApplication::platformName()!=u"wayland"_s){
        error(u"KDE overlays require a Wayland session."_s);
        return false;
    }
    QScreen *target=nullptr;
    for(auto *screen:QGuiApplication::screens())if(screen->name()==monitor)target=screen;
    if(!target){error(u"The animation monitor is disconnected."_s);return false;}
    if(active() && monitor!=m_monitor)clear();
    if(!active()){
        auto environment=QProcessEnvironment::systemEnvironment();
        if(isKde) {
            environment.insert(u"QT_QPA_PLATFORM"_s,u"wayland"_s);
            environment.insert(u"QT_WAYLAND_SHELL_INTEGRATION"_s,u"layer-shell"_s);
        } else {
            // For GNOME / XWayland / X11 sessions, use XCB for click-through transparent top-level windows
            environment.insert(u"QT_QPA_PLATFORM"_s,u"xcb"_s);
        }
        m_worker.setProcessEnvironment(environment);
        m_worker.start(QCoreApplication::applicationFilePath(),{u"--overlay-worker"_s,monitor});
        if(!m_worker.waitForStarted(3000)){error(u"Could not start the animation renderer."_s);return false;}
        m_monitor=monitor;
    }
    m_worker.readAllStandardOutput();
    m_worker.write(QJsonDocument(QJsonObject{{u"actors"_s,document.array()}}).toJson(QJsonDocument::Compact)+'\n');
    if(!m_worker.waitForBytesWritten(1000)){error(u"The renderer did not receive the configuration."_s);clear();return false;}
    QByteArray reply;
    for(int i=0;i<40 && !reply.contains("READY");++i){m_worker.waitForReadyRead(100);reply+=m_worker.readAllStandardOutput();if(m_worker.state()==QProcess::NotRunning)break;}
    if(!reply.contains("READY")){clear();error(u"The compositor did not confirm the overlay."_s);return false;}
    error({});emit activeChanged();return true;
}
int OverlayController::runWorker(int argc,char **argv){
    QGuiApplication app(argc,argv);
    app.setApplicationName(u"waniengine-actors"_s);
    QScreen *screen=nullptr;
    for(auto *candidate:app.screens())if(candidate->name()==app.arguments().value(2))screen=candidate;
    if(!screen && !app.screens().isEmpty()) screen = app.screens().first();
    if(!screen)return 3;
    QQuickView view;
    view.engine()->addImportPath(waniEngineRuntimeRoot());
    view.setTitle(u"WaniEngine — animations"_s);
    view.setFlags(Qt::FramelessWindowHint|Qt::WindowTransparentForInput|Qt::WindowDoesNotAcceptFocus|Qt::WindowStaysOnTopHint);
    view.setColor(Qt::transparent);
    view.setResizeMode(QQuickView::SizeRootObjectToView);
    view.setScreen(screen);
    view.setGeometry(screen->geometry());

#ifdef WANIENGINE_HAS_LAYER_SHELL_QT
    if (app.platformName() == u"wayland"_s) {
        auto *layer=LayerShellQt::Window::get(&view);
        if (layer) {
            layer->setLayer(LayerShellQt::Window::LayerOverlay);
            layer->setAnchors(LayerShellQt::Window::Anchors(LayerShellQt::Window::AnchorTop)|LayerShellQt::Window::AnchorBottom|LayerShellQt::Window::AnchorLeft|LayerShellQt::Window::AnchorRight);
            layer->setExclusiveZone(-1);
            layer->setKeyboardInteractivity(LayerShellQt::Window::KeyboardInteractivityNone);
            layer->setActivateOnShow(false);
            layer->setScope(u"waniengine-actors"_s);
            layer->setScreen(screen);
        }
    }
#endif
    if (app.platformName() == u"xcb"_s) {
        view.setProperty("_q_xcb_wm_window_type", "_NET_WM_WINDOW_TYPE_UTILITY");
    }

    view.resize(screen->size());
    view.setSource(QUrl(u"qrc:/qt/qml/WaniEngineManager/qml/ActorScene.qml"_s));
    if(view.status()!=QQuickView::Ready)return 4;
    fcntl(STDIN_FILENO,F_SETFL,fcntl(STDIN_FILENO,F_GETFL)|O_NONBLOCK);
    QByteArray pending;
    QSocketNotifier notifier(STDIN_FILENO,QSocketNotifier::Read);
    bool acknowledge=false;
    QObject::connect(&view,&QQuickWindow::frameSwapped,&app,[&]{
        if(acknowledge){acknowledge=false;QTextStream(stdout)<<"READY\n"<<Qt::flush;}
    });
    QObject::connect(&notifier,&QSocketNotifier::activated,&app,[&]{
        char buffer[65536]; const auto count=::read(STDIN_FILENO,buffer,sizeof(buffer));
        if(count==0){app.quit();return;}
        if(count<0)return;
        pending.append(buffer,count);
        if(pending.size()>maxMessage){app.exit(5);return;}
        while(pending.contains('\n')){
            const auto line=pending.left(pending.indexOf('\n'));pending.remove(0,line.size()+1);
            const auto document=QJsonDocument::fromJson(line);
            const auto actors=document.object().value(u"actors"_s);
            if(!document.isObject() || !actors.isArray() || !validActors(actors.toArray())){app.exit(6);return;}
            view.rootObject()->setProperty("actors",actors.toArray().toVariantList());
            acknowledge=true;view.show();view.update();
        }
    });
    // stdin closes on manager death. An overlay is never an orphan/autostart process.
    QObject::connect(&app,&QGuiApplication::screenRemoved,&app,[&](QScreen *removed){if(removed==screen)app.quit();});
    return app.exec();
}
