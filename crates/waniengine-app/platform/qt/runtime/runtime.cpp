#include "runtime.h"
#include <QSGSimpleTextureNode>
#include <QMouseEvent>
#include <QImage>
#include <QImageReader>
#include <QTimer>
#include <QDBusMessage>
#include <QDBusConnection>
#include <QDBusPendingCall>
#include <cmath>
#include <algorithm>
using namespace Qt::StringLiterals;
namespace {class OwnedTextureNode : public QSGSimpleTextureNode {public: ~OwnedTextureNode() override {delete texture();}};}
FrameClock::FrameClock(QObject *parent):QObject(parent){m_elapsed.start();m_timer.setTimerType(Qt::PreciseTimer);setFps(60);connect(&m_timer,&QTimer::timeout,this,&FrameClock::tick);}
void FrameClock::setFps(int value){value=std::clamp(value,15,1000);m_fps=value;m_timer.setInterval(std::max(1, 1000/value));emit fpsChanged();}
void FrameClock::setRunning(bool value){if(value==running())return;if(value)m_timer.start();else m_timer.stop();emit runningChanged();}
VideoFrameLimiter::VideoFrameLimiter(QObject *parent):QObject(parent){
    connect(&m_input,&QVideoSink::videoFrameChanged,this,[this](const QVideoFrame&frame){
        if(!frame.isValid()){m_latest={};m_pending=false;m_clock.setRunning(false);if(m_target)m_target->setVideoFrame({});return;}
        // Never queue frames while the target has no live RHI: they would be
        // handed to a sink whose graphics resources are already gone, which
        // crashes inside the FFmpeg decode thread.
        if(!m_target||!m_target->rhi()){m_latest={};m_pending=false;m_clock.setRunning(false);return;}
        m_latest=frame;m_pending=true;m_clock.setRunning(true);
    });
    connect(&m_clock,&FrameClock::tick,this,[this]{
        if(!m_target){m_clock.setRunning(false);return;}
        auto *rhi=m_target->rhi();
        if(!rhi){m_latest={};m_pending=false;m_clock.setRunning(false);return;}
        if(m_input.rhi()!=rhi)m_input.setRhi(rhi);
        if(m_pending){m_target->setVideoFrame(m_latest);m_pending=false;emit framePresented();}
    });
}
void VideoFrameLimiter::releaseFrame(){
    m_clock.setRunning(false);m_latest={};m_pending=false;
    if(m_target)m_target->setVideoFrame({});
    m_input.setVideoFrame({});
}
void VideoFrameLimiter::setTarget(QVideoSink*sink){
    if(m_target==sink)return;releaseFrame();if(m_target)disconnect(m_target,nullptr,this,nullptr);
    m_target=sink;
    // Only hand the decoder a real RHI. A null one makes the FFmpeg plugin
    // register a cleanup callback on nothing and crash on the next frame.
    m_input.setRhi(sink?sink->rhi():nullptr);
    if(sink)connect(sink,&QObject::destroyed,this,[this]{releaseFrame();m_input.setRhi(nullptr);m_clock.setRunning(false);});
    m_clock.setRunning(sink!=nullptr&&sink->rhi()!=nullptr&&m_latest.isValid());emit targetChanged();
}
CappedAnimation::CappedAnimation(QQuickItem *parent):QQuickItem(parent){
    setFlag(ItemHasContents);m_movie.setCacheMode(QMovie::CacheNone);
    connect(&m_movie,&QMovie::frameChanged,this,[this](int frame){
        m_pendingImage=m_movie.currentImage();m_dirty=true;
        if(m_paused)presentFrame();
        if(!m_repeat&&frame>0&&frame==m_movie.frameCount()-1){m_finished=true;m_movie.setPaused(true);}
    });
    connect(&m_movie,&QMovie::finished,this,[this]{
        if(m_repeat&&!m_paused&&m_movie.frameCount()>1) {
            QTimer::singleShot(0,this,[this]{if(m_repeat&&!m_paused&&!m_source.isEmpty())m_movie.start();});
        } else {m_finished=true;m_clock.setRunning(false);}
    });
    connect(&m_clock,&FrameClock::tick,this,&CappedAnimation::presentFrame);
    connect(this,&CappedAnimation::fillModeChanged,this,&QQuickItem::update);connect(this,&CappedAnimation::smoothChanged,this,&QQuickItem::update);
    m_clock.setRunning(false);
}
void CappedAnimation::presentFrame(){
    if(!m_dirty)return;
    // Move instead of copying: two full-size QImage buffers double the memory
    // held for every animation. The texture upload already reads from m_image.
    m_image=std::move(m_pendingImage);
    m_pendingImage={};
    m_presentedFrame=m_movie.currentFrameNumber();m_dirty=false;m_textureDirty=true;update();emit framePresented();
}
void CappedAnimation::updateDecodeSize(){
    if(m_size.isEmpty())return;
    QSize wanted=m_size;
    if(m_maximumSize.width()>0&&m_maximumSize.height()>0)wanted=m_size.scaled(m_maximumSize,Qt::KeepAspectRatio);
    wanted=QSize(std::max(1,std::min(wanted.width(),m_size.width())),std::max(1,std::min(wanted.height(),m_size.height())));
    if(m_movie.scaledSize()==wanted)return;
    const auto filename=m_movie.fileName();
    m_movie.stop();m_movie.setFileName({});m_movie.setFileName(filename);m_movie.setScaledSize(wanted);
    m_movie.jumpToFrame(0);presentFrame();m_movie.start();m_movie.setPaused(m_paused);
}
void CappedAnimation::setMaximumSourceSize(const QSize &size){if(size==m_maximumSize)return;m_maximumSize=size;updateDecodeSize();emit maximumSourceSizeChanged();}
void CappedAnimation::setSource(const QUrl&url){
    if(url==m_source)return;m_source=url;m_finished=false;m_movie.stop();
    m_pendingImage={};m_image={};m_size={};m_dirty=false;m_textureDirty=true;
    m_movie.setFileName({});
    if(!url.isEmpty()){
        QImageReader reader(url.toLocalFile());m_size=reader.size();
        m_movie.setFileName(url.toLocalFile());updateDecodeSize();m_movie.jumpToFrame(0);presentFrame();m_movie.start();m_movie.setPaused(m_paused);
    }
    m_clock.setRunning(!url.isEmpty()&&!m_paused&&m_movie.frameCount()!=1);update();emit sourceChanged();emit sourceSizeChanged();
}
void CappedAnimation::setSpeed(double value){m_movie.setSpeed(std::clamp(qRound(value*100),10,300));emit speedChanged();}
void CappedAnimation::setPaused(bool value){if(value==m_paused)return;m_paused=value;if(!value&&m_finished){m_finished=false;m_movie.stop();m_movie.start();}m_movie.setPaused(value);m_clock.setRunning(!value&&!m_source.isEmpty()&&m_movie.frameCount()!=1);presentFrame();emit pausedChanged();}
void CappedAnimation::setRepeat(bool value){m_repeat=value;if(value&&m_finished&&!m_paused){m_finished=false;m_movie.start();}emit repeatChanged();}
QSGNode* CappedAnimation::updatePaintNode(QSGNode*old,UpdatePaintNodeData*){
    auto *node=static_cast<QSGSimpleTextureNode*>(old);
    if(m_image.isNull()||!window()){delete node;return nullptr;}
    if(!node){node=new OwnedTextureNode;node->setOwnsTexture(false);m_textureDirty=true;}
    if(m_textureDirty){auto *previous=node->texture();auto *texture=window()->createTextureFromImage(m_image);node->setTexture(texture);if(previous&&previous!=texture)delete previous;m_textureDirty=false;}
    QRectF rect=boundingRect(),source(QPointF(0,0),m_image.size());
    if(m_fillMode==1){auto size=QSizeF(m_image.size());size.scale(rect.size(),Qt::KeepAspectRatio);rect=QRectF((width()-size.width())/2,(height()-size.height())/2,size.width(),size.height());}
    else if(m_fillMode==2){const double scale=std::max(width()/m_image.width(),height()/m_image.height());if(scale>0){QSizeF size(width()/scale,height()/scale);source=QRectF((m_image.width()-size.width())/2,(m_image.height()-size.height())/2,size.width(),size.height());}}
    else if(m_fillMode==6){rect=QRectF((width()-m_image.width())/2,(height()-m_image.height())/2,m_image.width(),m_image.height());}
    node->setRect(rect);node->setSourceRect(source);node->setFiltering(m_smooth?QSGTexture::Linear:QSGTexture::Nearest);return node;
}
void DesktopPointerObserver::setWindow(QQuickWindow*window){if(m_window==window)return;if(m_window)m_window->removeEventFilter(this);m_window=window;if(window)window->installEventFilter(this);emit windowChanged();}
bool DesktopPointerObserver::eventFilter(QObject*,QEvent*event){
    if(event->type()==QEvent::MouseButtonRelease){auto *mouse=static_cast<QMouseEvent*>(event);if(mouse->button()==Qt::LeftButton){auto message=QDBusMessage::createMethodCall(u"org.waniengine.WaniEngine"_s,u"/ActorAudio"_s,u"org.waniengine.ActorAudio"_s,u"desktopClick"_s);message<<mouse->globalPosition().x()<<mouse->globalPosition().y();QDBusConnection::sessionBus().asyncCall(message);}}
    return false;
}
