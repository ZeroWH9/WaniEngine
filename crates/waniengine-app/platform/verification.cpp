#include "verification.hpp"
#include "services.hpp"
#include "plasma_integration.h"
#include "overlay_controller.h"
#include "actor_audio_controller.h"
#include "media_metadata.h"
#include <QFile>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QProcess>
#include <QXmlStreamReader>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusReply>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJSValue>
#include <QMediaPlayer>
#include <QThread>
#include <QTextStream>
#include <QUrl>
#include <QWindow>
#include <functional>
#include <stdexcept>
using namespace Qt::StringLiterals;
namespace {
void require(bool value,const QString &message){if(!value)throw std::runtime_error(message.toStdString());}
QVariant value(QObject *root,const char *name){const auto result=root->property(name);return result.metaType()==QMetaType::fromType<QJSValue>()?result.value<QJSValue>().toVariant():result;}
void invoke(QObject *root,const char *method,QList<QVariant> args={}){
    bool ok=false;
    if(args.size()==0)ok=QMetaObject::invokeMethod(root,method);
    if(args.size()==1)ok=QMetaObject::invokeMethod(root,method,Q_ARG(QVariant,args[0]));
    if(args.size()==2)ok=QMetaObject::invokeMethod(root,method,Q_ARG(QVariant,args[0]),Q_ARG(QVariant,args[1]));
    if(args.size()==3)ok=QMetaObject::invokeMethod(root,method,Q_ARG(QVariant,args[0]),Q_ARG(QVariant,args[1]),Q_ARG(QVariant,args[2]));
    require(ok,u"Could not invoke QML function: "_s+QString::fromLatin1(method));
}
bool waitUntil(const std::function<bool()> &predicate,int timeout=15000){
    QElapsedTimer timer;timer.start();
    while(!predicate()&&timer.elapsed()<timeout){
        QCoreApplication::processEvents(QEventLoop::AllEvents,50);
        // The verifier occupies a timer callback. Explicitly deliver deferred
        // destruction as the normal outer event loop does between callbacks.
        QCoreApplication::sendPostedEvents(nullptr,QEvent::DeferredDelete);
        QThread::msleep(10);
    }
    return predicate();
}
QStringList wallpaperPlugins(){auto message=QDBusMessage::createMethodCall(u"org.kde.plasmashell"_s,u"/PlasmaShell"_s,u"org.kde.PlasmaShell"_s,u"evaluateScript"_s);message<<u"print(desktops().map(d=>String(d.wallpaperPlugin)).join('|'))"_s;QDBusReply<QString> reply=QDBusConnection::sessionBus().call(message);require(reply.isValid(),u"Plasma is unavailable in the isolated test session"_s);return reply.value().trimmed().split('|',Qt::SkipEmptyParts);}
int waniWallpaperCount(){int count=0;for(const auto &plugin:wallpaperPlugins())if(plugin.startsWith(u"org.waniengine.wallpaper.v"_s))++count;return count;}
qint64 rssKiB(qint64 pid){QFile file(u"/proc/%1/status"_s.arg(pid));if(!file.open(QIODevice::ReadOnly))return -1;for(const auto &line:file.readAll().split('\n'))if(line.startsWith("VmRSS:"))return line.mid(6).simplified().split(' ').first().toLongLong();return -1;}
QJsonObject gpuMemory(const QList<qint64> &pids){
    QProcess process;process.start(u"nvidia-smi"_s,{u"-q"_s,u"-x"_s});
    if(!process.waitForFinished(3000)||process.exitCode()!=0)return {};
    QJsonObject result;for(auto pid:pids)result.insert(QString::number(pid),0);
    QXmlStreamReader xml(process.readAllStandardOutput());
    while(!xml.atEnd()){xml.readNext();if(!xml.isStartElement()||xml.name()!=u"process_info")continue;qint64 pid=0;int memory=0;
        while(!xml.atEnd()){xml.readNext();if(xml.isEndElement()&&xml.name()==u"process_info")break;if(xml.isStartElement()&&xml.name()==u"pid")pid=xml.readElementText().toLongLong();else if(xml.isStartElement()&&xml.name()==u"used_memory")memory=xml.readElementText().split(' ').first().toInt();}
        if(pids.contains(pid))result.insert(QString::number(pid),result.value(QString::number(pid)).toInt()+memory);
    }return result;
}
}
int verify_backend(QObject *root,RustSceneStore &scene,RustPlaylistStore &playlists,PlasmaIntegration &plasma,OverlayController &overlays,ActorAudioController &audio,const QString &fixtures){
    QJsonArray checks;
    auto passed=[&](const QString &name){checks.append(name);QTextStream(stderr)<<"PASS "<<name<<'\n';};
    try {
        require(qEnvironmentVariable("WANIENGINE_ISOLATED_TEST_ROOT")==fixtures && fixtures.startsWith(u"/tmp/waniengine-rust-test."_s),u"Desktop verification must run in the isolated test script"_s);
        const QStringList original=wallpaperPlugins();
        const QVariantList monitors=plasma.monitors();
        require(monitors.size()>=2 && original.size()>=2,u"The isolated desktop must expose two independent monitors"_s);
        const QString firstMonitor=monitors[0].toMap().value(u"name"_s).toString();
        const QString secondMonitor=monitors[1].toMap().value(u"name"_s).toString();
        require(!firstMonitor.isEmpty() && !secondMonitor.isEmpty() && firstMonitor!=secondMonitor,u"The virtual monitors need distinct stable names"_s);
        const auto video=QUrl::fromLocalFile(fixtures+u"/video.mp4"_s).toString();
        const auto actor=QUrl::fromLocalFile(fixtures+u"/animation.gif"_s).toString();
        const auto sound=QUrl::fromLocalFile(fixtures+u"/sound.wav"_s).toString();
        invoke(root,"importFiles",{QVariantList{video},u"wallpaper"_s});
        auto assets=value(root,"assets").toList();require(assets.size()==1,u"Video import failed"_s);const QString wallpaper=assets.first().toMap().value(u"url"_s).toString();
        require(!scene.lastError().size(),scene.lastError());require(wallpaper!=video,u"Import did not copy the source into the preview library"_s);passed(u"Rust library import and metadata"_s);
        invoke(root,"updateAsset",{wallpaper,u"audioEnabled"_s,true});invoke(root,"updateAsset",{wallpaper,u"volume"_s,35});
        invoke(root,"selectMonitor",{firstMonitor,false});
        invoke(root,"applySelected");require(waitUntil([&]{return !root->property("busyApplying").toBool();},180000),u"Video preparation timed out"_s);require(plasma.isActive(),plasma.lastError());require(waniWallpaperCount()==1,u"Plasma did not apply the isolated wallpaper to exactly one output"_s);passed(u"Async video optimization and real Plasma application"_s);
        invoke(root,"selectMonitor",{secondMonitor,false});invoke(root,"applySelected");require(waitUntil([&]{return !root->property("busyApplying").toBool();},180000),u"Second-monitor video preparation timed out"_s);
        const auto twoMonitorState=QJsonDocument::fromJson(scene.loadScene().toUtf8()).object();const auto wallpaperMap=twoMonitorState.value(u"wallpapersByMonitor"_s).toObject();
        require(wallpaperMap.value(firstMonitor).toObject().value(u"url"_s).toString()==wallpaper && wallpaperMap.value(secondMonitor).toObject().value(u"url"_s).toString()==wallpaper,u"Wallpaper assignments were not persisted independently for both monitors"_s);
        require(waniWallpaperCount()==2,u"The two-monitor test wallpaper was not applied to both outputs"_s);
        passed(u"Independent per-monitor wallpaper persistence and application"_s);
        root->setProperty("restoreRequested",true);root->setProperty("restoredWallpapers",QVariantMap{});invoke(root,"restoreSavedWallpapers");
        require(waitUntil([&]{return !root->property("busyApplying").toBool() && value(root,"restoredWallpapers").toMap().size()>=2;},180000),u"Saved wallpapers were not restored on both monitors"_s);
        require(waniWallpaperCount()==2,u"Startup restoration did not reapply both monitor wallpapers"_s);
        passed(u"Startup restoration reapplies each saved monitor wallpaper"_s);
        invoke(root,"selectMonitor",{firstMonitor,false});
        require(assets.first().toMap().value(u"width"_s).toInt()==3840,u"The fixture must be a native 4K video"_s);
        const auto optimized=describeLocalMedia(QUrl(root->property("backgroundPlaybackUrl").toString()),fixtures+u"/thumbs"_s);
        require(optimized.value(u"width"_s).toInt()==1920&&optimized.value(u"height"_s).toInt()==1080,u"The applied 4K video was not reduced to FullHD"_s);
        auto *limiter=root->findChild<QObject*>(u"previewFrameLimiter"_s);require(limiter,u"Preview frame limiter missing"_s);
        require(waitUntil([&]{return limiter->property("frameSize").toSize()==QSize(1920,1080);},180000),u"The preview decoder did not produce FullHD frames"_s);
        auto *quick=qobject_cast<QQuickWindow*>(root);quick->hide();require(waitUntil([&]{return limiter->property("frameSize").toSize().isEmpty();}),u"Hidden preview retained its decoded frame"_s);
        quick->show();require(waitUntil([&]{return limiter->property("frameSize").toSize()==QSize(1920,1080);}),u"The preview did not resume after showing the window"_s);
        QQmlComponent component(qmlEngine(root));component.setData("import QtQuick\nimport WaniEngine.Runtime 1.0 as Engine\nEngine.CappedAnimation { width:128; height:128; maximumSourceSize:Qt.size(64,64); paused:true }",QUrl());
        auto *animation=qobject_cast<QQuickItem*>(component.create());require(animation,component.errorString());animation->setParentItem(quick->contentItem());
        animation->setProperty("source",QUrl::fromLocalFile(fixtures+u"/large.png"_s));
        require(waitUntil([&]{return animation->property("decodedSize").toSize()==QSize(64,64);}),u"Character texture did not honor its decode limit"_s);
        require(animation->property("sourceSize").toSize()==QSize(2048,2048),u"Native character geometry was lost"_s);
        for(int i=0;i<20;++i){animation->setProperty("maximumSourceSize",QSize(128,128));require(animation->property("decodedSize").toSize()==QSize(128,128),u"Animation resize failed"_s);animation->setProperty("maximumSourceSize",QSize(64,64));}
        animation->setProperty("source",QUrl());require(animation->property("decodedSize").toSize().isEmpty(),u"Cleared animation retained decoded pixels"_s);delete animation;
        passed(u"4K to FullHD application and preview, texture bounds and hidden-frame cleanup"_s);
        invoke(root,"updateAsset",{wallpaper,u"fit"_s,u"contain"_s});invoke(root,"updateAsset",{wallpaper,u"loop"_s,false});waitUntil([]{return false;},180);require(plasma.lastError().isEmpty(),plasma.lastError());passed(u"Live wallpaper fit, playback and audio options"_s);
        invoke(root,"addSelectedToPlaylist");const auto playlist=QUrl::fromLocalFile(fixtures+u"/playlist.json"_s).toString();invoke(root,"savePlaylistFile",{playlist});require(!playlists.load(QUrl(playlist)).isEmpty(),playlists.lastError());invoke(root,"loadPlaylistFile",{playlist});require(value(root,"playlist").toList().size()==1,u"Playlist reload failed"_s);passed(u"Rust playlist save and reload"_s);
        invoke(root,"importFiles",{QVariantList{actor},u"character"_s});assets=value(root,"assets").toList();require(assets.size()==2,u"Character import failed"_s);const QString character=root->property("selectedUrl").toString();
        invoke(root,"navigate",{u"installed"_s});require(value(root,"visibleAssets").toList().size()==2,u"Library does not show both wallpapers and characters"_s);
        root->setProperty("selectedUrl",character);require(waitUntil([&]{return !root->property("preparingPreview").toBool();}),u"Previous video preparation did not clear"_s);
        auto *gifPreview=root->findChild<QObject*>(u"animationPreview"_s);require(gifPreview,u"GIF preview missing"_s);
        invoke(root,"togglePreview");require(gifPreview->property("paused").toBool(),u"GIF preview did not pause"_s);invoke(root,"togglePreview");require(!gifPreview->property("paused").toBool(),u"GIF preview did not resume"_s);
        root->setProperty("contentFilter",u"character"_s);require(value(root,"visibleAssets").toList().size()==1,u"The library character filter is broken"_s);invoke(root,"navigate",{u"animations"_s});require(root->property("selectedUrl").toString()==character,u"Character selection was not preserved"_s);passed(u"Complete library and working character filter"_s);
        invoke(root,"addAudioFiles",{QVariantList{sound}});invoke(root,"updateAudio",{u"enabled"_s,true});
        int sounds=0;QObject::connect(&audio,&ActorAudioController::soundStarted,root,[&](const QString&,const QString&){++sounds;});
        invoke(root,"applySelected");require(overlays.active(),overlays.lastError());require(waitUntil([&]{return sounds>0;}),u"Character audio did not start"_s);require(!audio.lastError().size(),audio.lastError());passed(u"Click-through Wayland overlay and per-character audio"_s);
        invoke(root,"updateAsset",{character,u"opacity"_s,70});invoke(root,"updateAsset",{character,u"flipX"_s,true});waitUntil([]{return false;},180);require(overlays.active()&&overlays.lastError().isEmpty(),overlays.lastError());passed(u"Live character transform updates"_s);
        auto *window=qobject_cast<QWindow*>(root);require(window,u"The manager window is missing"_s);
        window->showFullScreen();window->requestActivate();require(waitUntil([&]{return root->property("automaticPause").toBool();},20000),u"Fullscreen pause policy did not activate"_s);
        window->showNormal();require(waitUntil([&]{return !root->property("automaticPause").toBool();}),u"Playback did not resume after leaving fullscreen"_s);
        auto settings=value(root,"settings").toMap();settings.insert(u"pauseMaximized"_s,true);root->setProperty("settings",settings);
        window->showMaximized();window->requestActivate();require(waitUntil([&]{return root->property("automaticPause").toBool();}),u"Maximized pause policy did not activate"_s);
        window->showNormal();require(waitUntil([&]{return !root->property("automaticPause").toBool();}),u"Playback did not resume after leaving maximized mode"_s);passed(u"KDE fullscreen and maximized pause/resume policies"_s);
        QJsonArray memory;
        const int stressSeconds=qEnvironmentVariableIntValue("WANIENGINE_STRESS_SECONDS");
        if(stressSeconds>0){
            invoke(root,"navigate",{u"installed"_s});root->setProperty("selectedUrl",wallpaper);invoke(root,"updateAsset",{wallpaper,u"loop"_s,true});
            require(waitUntil([&]{return limiter->property("frameSize").toSize()==QSize(1920,1080);}),u"Stress preview did not resume"_s);
            waitUntil([]{return false;},5000);
            const auto manager=QCoreApplication::applicationPid(),worker=overlays.workerPid(),shell=qEnvironmentVariable("WANIENGINE_TEST_PLASMA_PID").toLongLong();
            QElapsedTimer elapsed;elapsed.start();
            while(elapsed.elapsed()<qBound(30,stressSeconds,600)*1000){
                invoke(root,"updateAsset",{character,u"opacity"_s,memory.size()%2?70:85});
                waitUntil([]{return false;},2000);
                memory.append(QJsonObject{{u"seconds"_s,elapsed.elapsed()/1000.0},{u"managerRssKiB"_s,rssKiB(manager)},{u"actorRssKiB"_s,rssKiB(worker)},{u"plasmaRssKiB"_s,rssKiB(shell)},{u"gpuMiBByPid"_s,gpuMemory({manager,worker,shell})}});
            }
            for(const auto &key:{u"managerRssKiB"_s,u"actorRssKiB"_s,u"plasmaRssKiB"_s}){
                qint64 early=0,late=0;const int count=qMax(1,memory.size()/3);
                for(int i=0;i<count;++i){early+=memory[i].toObject().value(key).toInteger();late+=memory[memory.size()-count+i].toObject().value(key).toInteger();}
                require((late-early)/count<48*1024,u"Sustained resident-memory growth: "_s+key);
            }
            passed(u"Bounded resident memory during repeated wallpaper and Wayland character playback"_s);
        }
        const auto saved=QJsonDocument::fromJson(scene.loadScene().toUtf8()).object();require(saved.value(u"assets"_s).toArray().size()==2,u"Saved library state is incomplete"_s);passed(u"Persisted complete library state"_s);
        root->setProperty("shuttingDown",true);overlays.clear();audio.clear();require(plasma.restore(),plasma.lastError());require(wallpaperPlugins()==original,u"The original desktop wallpaper was not restored on every monitor"_s);passed(u"Overlay cleanup and multi-monitor desktop restoration"_s);
        QTextStream(stdout)<<QJsonDocument(QJsonObject{{u"ok"_s,true},{u"checks"_s,checks},{u"memorySamples"_s,memory},{u"managerPid"_s,QCoreApplication::applicationPid()}}).toJson(QJsonDocument::Compact)<<'\n';return 0;
    } catch(const std::exception &error){root->setProperty("shuttingDown",true);overlays.clear();audio.clear();plasma.restore();QTextStream(stderr)<<"FAIL "<<error.what()<<'\n';return 1;}
}
