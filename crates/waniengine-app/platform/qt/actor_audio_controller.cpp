#include "actor_audio_controller.h"
#include <QAudioOutput>
#include <QMediaPlayer>
#include <QMediaDevices>
#include <QAudioDevice>
#include <QTimer>
#include <QJsonDocument>
#include <QJsonArray>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusReply>
#include <QDBusMessage>
#include <QGuiApplication>
#include <QScreen>
#include <QFileInfo>
#include <QUrl>
#include <QRandomGenerator>
#include <QTransform>
#include <cmath>
#include <algorithm>
using namespace Qt::StringLiterals;
ActorAudioController::ActorAudioController(QObject*parent):QObject(parent){
    auto bus=QDBusConnection::sessionBus();m_registered=bus.registerService(u"org.waniengine.WaniEngine"_s);
    if(m_registered)bus.registerObject(u"/ActorAudio"_s,this,QDBusConnection::ExportScriptableSlots);
    auto *devices=new QMediaDevices(this);connect(devices,&QMediaDevices::audioOutputsChanged,this,[this]{for(auto*voice:m_voices)updateOutput(voice);});
}
ActorAudioController::~ActorAudioController(){clear();if(m_registered){auto bus=QDBusConnection::sessionBus();bus.unregisterObject(u"/ActorAudio"_s);bus.unregisterService(u"org.waniengine.WaniEngine"_s);}}
void ActorAudioController::clear(){for(auto *voice:m_voices){voice->timer->stop();voice->player->stop();delete voice->timer;delete voice->player;delete voice->output;delete voice;}m_voices.clear();}
void ActorAudioController::error(const QString&text){m_error=text;emit lastErrorChanged();}
void ActorAudioController::updateOutput(Voice*voice){
    auto chosen=QMediaDevices::defaultAudioOutput();const auto id=m_options.value(u"deviceId"_s).toString().toUtf8();for(const auto&device:QMediaDevices::audioOutputs())if(device.id()==id)chosen=device;
    voice->output->setDevice(chosen);voice->output->setVolume(std::clamp(voice->audio.value(u"volume"_s).toDouble(100)/100,0.0,1.0)*std::clamp(voice->actor.value(u"volume"_s).toDouble(50)/100,0.0,1.0)*std::clamp(m_options.value(u"globalVolume"_s).toDouble(100)/100,0.0,1.0));
}
void ActorAudioController::configure(const QString&actors,const QString&options,const QString&monitor){
    auto data=QJsonDocument::fromJson(actors.toUtf8());if(!data.isArray()||data.array().size()>16)return;
    m_options=QJsonDocument::fromJson(options.toUtf8()).object();m_monitor=monitor;QStringList keep;
    QJsonArray schedules;
    for(const auto &value:data.array()){
        const auto actor=value.toObject(),audio=actor.value(u"audio"_s).toObject();
        for(const auto &entry:audio.value(u"files"_s).toArray()){
            auto track=entry.toObject();
            if(!track.value(u"playback"_s).isObject())continue;
            const auto playback=track.take(u"playback"_s).toObject();
            if(!playback.value(u"enabled"_s).toBool(true))continue;
            auto schedule=audio;for(auto it=playback.begin();it!=playback.end();++it)schedule.insert(it.key(),it.value());
            schedule.insert(u"enabled"_s,audio.value(u"enabled"_s));schedule.insert(u"files"_s,QJsonArray{track});schedule.insert(u"random"_s,false);
            auto item=actor;item.insert(u"audio"_s,schedule);item.insert(u"voiceKey"_s,actor.value(u"url"_s).toString()+u"|"_s+track.value(u"url"_s).toString());schedules.append(item);
        }
    }
    for(const auto&value:schedules){
        const auto actor=value.toObject(),audio=actor.value(u"audio"_s).toObject();const QString key=actor.value(u"voiceKey"_s).toString(actor.value(u"url"_s).toString());
        if(!audio.value(u"enabled"_s).toBool()||audio.value(u"files"_s).toArray().isEmpty())continue;keep<<key;
        const bool fresh=!m_voices.contains(key);auto *voice=fresh?new Voice:m_voices.value(key);
        const bool changed=voice->audio.value(u"mode"_s)!=audio.value(u"mode"_s)||voice->audio.value(u"files"_s)!=audio.value(u"files"_s);
        const bool intervalChanged=voice->audio.value(u"intervalSeconds"_s)!=audio.value(u"intervalSeconds"_s);
        voice->actor=actor;voice->audio=audio;
        if(fresh){voice->output=new QAudioOutput(this);voice->player=new QMediaPlayer(this);voice->player->setObjectName(u"actorAudio:"_s+key);voice->player->setAudioOutput(voice->output);voice->timer=new QTimer(this);
            connect(voice->timer,&QTimer::timeout,this,[this,voice]{play(voice);});
            connect(voice->player,&QMediaPlayer::mediaStatusChanged,this,[this,voice](QMediaPlayer::MediaStatus status){if(status==QMediaPlayer::EndOfMedia&&voice->audio.value(u"mode"_s).toString()==u"loop"_s)QTimer::singleShot(0,this,[this,voice]{if(m_voices.values().contains(voice))play(voice);});});
            connect(voice->player,&QMediaPlayer::errorOccurred,this,[this](QMediaPlayer::Error,const QString&text){error(u"Character audio: "_s+text);});m_voices.insert(key,voice);
        }
        updateOutput(voice);
        if(fresh||changed){voice->timer->stop();voice->player->stop();voice->next=0;const auto mode=audio.value(u"mode"_s).toString(u"loop"_s);if(mode==u"loop"_s||mode==u"once"_s||(mode==u"interval"_s&&audio.value(u"startImmediately"_s).toBool(true)))play(voice);}
        if(fresh||changed||intervalChanged){voice->timer->stop();if(audio.value(u"mode"_s).toString()==u"interval"_s)voice->timer->start(qRound(std::clamp(audio.value(u"intervalSeconds"_s).toDouble(30),1.0,86400.0)*1000));}
    }
    for(auto it=m_voices.begin();it!=m_voices.end();){if(!keep.contains(it.key())){auto *voice=it.value();voice->timer->stop();voice->player->stop();delete voice->timer;delete voice->player;delete voice->output;delete voice;it=m_voices.erase(it);}else ++it;}
}
void ActorAudioController::play(Voice*voice,bool forced,int requested){
    if(!forced&&voice->player->playbackState()==QMediaPlayer::PlayingState)return;
    const auto tracks=voice->audio.value(u"files"_s).toArray();if(tracks.isEmpty())return;
    const int index=requested>=0?requested:voice->audio.value(u"random"_s).toBool()?QRandomGenerator::global()->bounded(tracks.size()):voice->next++%tracks.size();
    const QUrl source(tracks.at(index).toObject().value(u"url"_s).toString());if(!source.isLocalFile()||!QFileInfo(source.toLocalFile()).isFile()){error(u"The audio file is missing."_s);return;}
    voice->player->stop();voice->player->setSource(source);voice->player->setPlaybackRate(1);voice->player->play();voice->lastPlay.restart();emit soundStarted(voice->actor.value(u"url"_s).toString(),source.toString());
}
void ActorAudioController::testSound(const QString&url){for(auto *voice:m_voices)if(voice->actor.value(u"url"_s).toString()==url){play(voice,true);return;}}
void ActorAudioController::testTrack(const QString&url,const QString&track){for(auto *voice:m_voices){if(voice->actor.value(u"url"_s).toString()!=url)continue;const auto files=voice->audio.value(u"files"_s).toArray();for(int i=0;i<files.size();++i)if(files[i].toObject().value(u"url"_s).toString()==track){play(voice,true,i);return;}}}
void ActorAudioController::desktopClick(double x,double y){
    if(calledFromDBus()){const QDBusReply<QString> owner=QDBusConnection::sessionBus().interface()->serviceOwner(u"org.kde.plasmashell"_s);if(!owner.isValid()||message().service()!=owner.value())return;}
    if(!std::isfinite(x)||!std::isfinite(y))return;QScreen *screen=nullptr;for(auto*candidate:QGuiApplication::screens())if(candidate->name()==m_monitor)screen=candidate;if(!screen)return;
    for(auto *voice:m_voices){if(voice->audio.value(u"mode"_s).toString()!=u"click"_s)continue;const auto a=voice->actor;const QRect geometry=screen->geometry();
        const double width=std::min(double(geometry.width()),a.value(u"width"_s).toDouble(1)*std::clamp(a.value(u"scale"_s).toDouble(100),10.0,300.0)/100),height=width*a.value(u"height"_s).toDouble(1)/std::max(1.0,a.value(u"width"_s).toDouble(1));
        const double cx=geometry.x()+geometry.width()*a.value(u"x"_s).toDouble(.5),cy=geometry.y()+geometry.height()*a.value(u"y"_s).toDouble(.96)-height/2;
        QTransform transform;transform.rotate(-a.value(u"rotation"_s).toDouble());const QPointF point=transform.map(QPointF(x-cx,y-cy));
        const double dx=std::max(0.0,std::abs(point.x())-width/2),dy=std::max(0.0,std::abs(point.y())-height/2),radius=std::clamp(voice->audio.value(u"radiusPixels"_s).toDouble(40),0.0,500.0);
        const double cooldown=std::clamp(voice->audio.value(u"cooldownSeconds"_s).toDouble(2),0.0,3600.0);
        if(dx*dx+dy*dy<=radius*radius&&(!voice->lastPlay.isValid()||voice->lastPlay.elapsed()>=cooldown*1000))play(voice);
    }
}
