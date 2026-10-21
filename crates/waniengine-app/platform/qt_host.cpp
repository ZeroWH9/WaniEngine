#include "qt_host.hpp"
#include "services.hpp"
#include "verification.hpp"
#include "actor_audio_controller.h"
#include "application_controller.h"
#include "desktop_integration.h"
#include "gnome_integration.h"
#include "media_optimizer.h"
#include "overlay_controller.h"
#include "package_validator.h"
#include "plasma_integration.h"
#include <QApplication>
#include <QDir>
#include <QIcon>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QLockFile>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QStandardPaths>
#include <QTextStream>
#include <QTimer>
#include <QMenu>
#include <QLocale>
#include <vector>
using namespace Qt::StringLiterals;

namespace waniengine {
int run_manager(rust::Str arguments,rust::Str runtime_root) {
    const auto args=QJsonDocument::fromJson(QByteArray(arguments.data(),static_cast<qsizetype>(arguments.size()))).array();
    std::vector<QByteArray> storage;storage.reserve(args.size());for(const auto &arg:args)storage.push_back(arg.toString().toLocal8Bit());
    std::vector<char*> argv;for(auto &arg:storage)argv.push_back(arg.data());argv.push_back(nullptr);int argc=static_cast<int>(storage.size());
    if(argc>1 && storage[1]=="--guardian")return PlasmaIntegration::runGuardian(argc,argv.data());
    if(argc>1 && storage[1]=="--overlay-worker")return OverlayController::runWorker(argc,argv.data());
    if(argc>1 && storage[1]=="--gnome-wallpaper-worker")return GnomeIntegration::runWallpaperWorker(argc,argv.data());
    if(qEnvironmentVariableIsEmpty("QT_QPA_PLATFORMTHEME")&&qEnvironmentVariable("XDG_CURRENT_DESKTOP").contains(u"KDE"_s))qputenv("QT_QPA_PLATFORMTHEME","kde");
    QLocale::setDefault(QLocale(QLocale::English,QLocale::UnitedStates));
    QApplication app(argc,argv.data());QQuickStyle::setStyle(u"Basic"_s);
    app.setApplicationName(u"waniengine"_s);app.setOrganizationName(u"WaniEngine"_s);app.setApplicationDisplayName(u"WaniEngine"_s);app.setApplicationVersion(QString::fromUtf8(WANIENGINE_VERSION));app.setDesktopFileName(u"org.waniengine.WaniEngine"_s);app.setWindowIcon(QIcon(u":/icons/waniengine.png"_s));app.setQuitOnLastWindowClosed(false);
    const auto options=app.arguments();
    if(options.contains(u"--validate-wallpaper"_s)){const auto result=validateWallpaperPackage(options.value(options.indexOf(u"--validate-wallpaper"_s)+1));QTextStream(stdout)<<QJsonDocument(result).toJson(QJsonDocument::Compact)<<'\n';return result.value(u"ok"_s).toBool()?0:1;}
    const auto runtime=QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
    const bool inspection=options.contains(u"--capture"_s)||options.contains(u"--diagnostics"_s);
    if(inspection)app.setApplicationName(u"waniengine-inspection-%1"_s.arg(app.applicationPid()));
    const auto socketPath=runtime+(inspection ? u"/waniengine-inspection-%1.sock"_s.arg(app.applicationPid()) : u"/waniengine-control.sock"_s);
    QLockFile lock(runtime+u"/waniengine.lock"_s);lock.setStaleLockTime(0);
    if(!options.contains(u"--diagnostics"_s)&&!options.contains(u"--capture"_s)&&!lock.tryLock(100)){
        QLocalSocket socket;socket.connectToServer(socketPath);if(!socket.waitForConnected(1500)){QTextStream(stderr)<<"The running Rust manager is not responding.\n";return 1;}
        socket.write(options.contains(u"--quit"_s)?"{\"action\":\"exit\"}\n":"{\"action\":\"open\"}\n");socket.waitForBytesWritten(1000);return 0;
    }
    if(options.contains(u"--quit"_s))return 0;
    QString stateHome=qEnvironmentVariable("XDG_STATE_HOME");if(stateHome.isEmpty())stateHome=QDir::homePath()+u"/.local/state"_s;
    QString dataRoot=qEnvironmentVariable("WANIENGINE_DATA_ROOT");if(dataRoot.isEmpty())dataRoot=stateHome+u"/WaniEngine/waniengine/library"_s;
    const int dataIndex=options.indexOf(u"--data-dir"_s);if(dataIndex>=0)dataRoot=options.value(dataIndex+1);
    QString installedScene=qEnvironmentVariable("WANIENGINE_INSTALLED_SCENE");if(installedScene.isEmpty())installedScene=stateHome+u"/WaniEngine/waniengine/library/scene.json"_s;
    RustSceneStore scene(dataRoot,installedScene);RustPlaylistStore playlists(dataRoot);VideoPreparation media;DesktopIntegration desktop;OverlayController overlays;ApplicationController controller(&desktop,socketPath);RustApplicationController application(&controller);ActorAudioController audio;NativeFilePicker picker;
    for(auto *widget:QApplication::allWidgets())if(auto *menu=qobject_cast<QMenu*>(widget))for(auto *action:menu->actions())if(action->objectName()==u"trayOpen"_s)action->setText(u"Open WaniEngine"_s);
    if(options.contains(u"--diagnostics"_s)){
        const auto state=QJsonDocument::fromJson(scene.loadScene().toUtf8()).object();QJsonArray monitors;for(const auto &value:desktop.monitors())monitors.append(QJsonObject::fromVariantMap(value.toMap()));
        QTextStream(stdout)<<QJsonDocument(QJsonObject{{u"assets"_s,state.value(u"assets"_s).toArray().size()},{u"dataRoot"_s,dataRoot},{u"installedScene"_s,installedScene},{u"monitors"_s,monitors},{u"samples"_s,QJsonArray::fromVariantList(scene.sampleLevels())},{u"error"_s,scene.lastError()},{u"nativePicker"_s,true}}).toJson(QJsonDocument::Compact)<<'\n';return scene.lastError().isEmpty()?0:1;
    }
    QQmlApplicationEngine engine;engine.addImportPath(QString::fromUtf8(runtime_root.data(),static_cast<qsizetype>(runtime_root.size())));
    auto *context=engine.rootContext();context->setContextProperty(u"sceneStore"_s,&scene);context->setContextProperty(u"playlistStore"_s,&playlists);context->setContextProperty(u"mediaOptimizer"_s,&media);context->setContextProperty(u"plasmaIntegration"_s,&desktop);context->setContextProperty(u"overlayController"_s,&overlays);context->setContextProperty(u"applicationController"_s,&application);context->setContextProperty(u"actorAudioController"_s,&audio);context->setContextProperty(u"nativeFilePicker"_s,&picker);context->setContextProperty(u"startHidden"_s,options.contains(u"--background"_s));context->setContextProperty(u"resumeWallpaper"_s,options.contains(u"--resume-wallpaper"_s));
    engine.load(QUrl(u"qrc:/qt/qml/WaniEngine/qml/generated/Main_redesign.qml"_s));if(engine.rootObjects().isEmpty())return 1;
    auto *window=qobject_cast<QQuickWindow*>(engine.rootObjects().first());controller.setWindow(window);picker.setWindow(window);window->setPersistentGraphics(true);window->setPersistentSceneGraph(true);
    QObject::connect(&app,&QCoreApplication::aboutToQuit,&app,[&]{window->setProperty("shuttingDown",true);overlays.clear();audio.clear();desktop.restore();});
    const int verification=options.indexOf(u"--verify-backend"_s);
    if(verification>=0)QTimer::singleShot(0,&app,[&]{
        PlasmaIntegration plasma;
        app.exit(verify_backend(window,scene,playlists,plasma,overlays,audio,options.value(verification+1)));
    });
    const int capture=options.indexOf(u"--capture"_s);
    if(capture>=0){const auto screen=options.value(options.indexOf(u"--page"_s)+1,u"installed"_s);QMetaObject::invokeMethod(window,"navigate",Q_ARG(QVariant,QVariant(screen)));QTimer::singleShot(2500,&app,[&,capture]{const bool ok=window->grabWindow().save(options.value(capture+1));app.exit(ok?0:1);});}
    return app.exec();
}
}
