#pragma once
#include <QTimer>
#include <QElapsedTimer>
#include <QMovie>
#include <QQuickItem>
#include <QVideoSink>
#include <QVideoFrame>
#include <QPointer>
#include <QQuickWindow>

class FrameClock : public QObject {
    Q_OBJECT
    Q_PROPERTY(int fps READ fps WRITE setFps NOTIFY fpsChanged)
    Q_PROPERTY(bool running READ running WRITE setRunning NOTIFY runningChanged)
    Q_PROPERTY(double seconds READ seconds NOTIFY tick)
public:
    explicit FrameClock(QObject *parent=nullptr);
    int fps() const {return m_fps;} void setFps(int value);
    bool running() const {return m_timer.isActive();} void setRunning(bool value);
    double seconds() const {return m_elapsed.nsecsElapsed()/1e9;}
signals: void fpsChanged(); void runningChanged(); void tick();
private: int m_fps=60; QTimer m_timer; QElapsedTimer m_elapsed;
};
class VideoFrameLimiter : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVideoSink* videoSink READ videoSink CONSTANT)
    Q_PROPERTY(QVideoSink* target READ target WRITE setTarget NOTIFY targetChanged)
    Q_PROPERTY(int fps READ fps WRITE setFps NOTIFY fpsChanged)
    Q_PROPERTY(QSize frameSize READ frameSize NOTIFY framePresented)
public:
    explicit VideoFrameLimiter(QObject *parent=nullptr);
    Q_INVOKABLE QVideoSink* videoSink(){return &m_input;} QVideoSink* target() const{return m_target;}
    void setTarget(QVideoSink *sink);
    QSize frameSize() const{return m_latest.isValid()?m_latest.size():QSize();}
    Q_INVOKABLE void releaseFrame();
    int fps() const{return m_clock.fps();} void setFps(int fps){m_clock.setFps(fps);emit fpsChanged();}
signals: void targetChanged(); void fpsChanged(); void framePresented();
private: QVideoSink m_input; QPointer<QVideoSink> m_target; QVideoFrame m_latest; bool m_pending=false; FrameClock m_clock;
};
class CappedAnimation : public QQuickItem {
    Q_OBJECT
    Q_PROPERTY(QUrl source READ source WRITE setSource NOTIFY sourceChanged)
    Q_PROPERTY(QSize sourceSize READ sourceSize NOTIFY sourceSizeChanged)
    Q_PROPERTY(QSize maximumSourceSize READ maximumSourceSize WRITE setMaximumSourceSize NOTIFY maximumSourceSizeChanged)
    Q_PROPERTY(QSize decodedSize READ decodedSize NOTIFY framePresented)
    Q_PROPERTY(int frameCount READ frameCount NOTIFY sourceSizeChanged)
    Q_PROPERTY(int currentFrame READ currentFrame NOTIFY framePresented)
    Q_PROPERTY(double speed READ speed WRITE setSpeed NOTIFY speedChanged)
    Q_PROPERTY(bool paused READ paused WRITE setPaused NOTIFY pausedChanged)
    Q_PROPERTY(bool repeat READ repeat WRITE setRepeat NOTIFY repeatChanged)
    Q_PROPERTY(int fps READ fps WRITE setFps NOTIFY fpsChanged)
    Q_PROPERTY(int fillMode MEMBER m_fillMode NOTIFY fillModeChanged)
    Q_PROPERTY(bool smooth MEMBER m_smooth NOTIFY smoothChanged)
public:
    explicit CappedAnimation(QQuickItem *parent=nullptr);
    QUrl source() const{return m_source;} void setSource(const QUrl&url);
    QSize sourceSize() const{return m_size;} int frameCount() const{return m_movie.frameCount();}
    QSize maximumSourceSize() const{return m_maximumSize;} void setMaximumSourceSize(const QSize &size);
    QSize decodedSize() const{return m_image.size();}
    int currentFrame() const{return m_presentedFrame;} double speed() const{return m_movie.speed()/100.0;}
    void setSpeed(double speed); bool paused() const{return m_paused;} void setPaused(bool paused);
    bool repeat() const{return m_repeat;} void setRepeat(bool repeat);
    int fps() const{return m_clock.fps();} void setFps(int fps){m_clock.setFps(fps);emit fpsChanged();}
signals: void sourceChanged(); void sourceSizeChanged(); void maximumSourceSizeChanged(); void speedChanged(); void pausedChanged(); void repeatChanged(); void fpsChanged(); void fillModeChanged(); void smoothChanged(); void framePresented();
protected: QSGNode* updatePaintNode(QSGNode*,UpdatePaintNodeData*) override;
private: void presentFrame(); void updateDecodeSize(); QUrl m_source; QSize m_size,m_maximumSize; QMovie m_movie; FrameClock m_clock; QImage m_pendingImage,m_image;
    int m_presentedFrame=0,m_fillMode=1; bool m_paused=false,m_repeat=true,m_smooth=true,m_dirty=false,m_textureDirty=false,m_finished=false;
};
// Passive observation of the existing desktop window. Returning false preserves delivery.
class DesktopPointerObserver : public QObject {
    Q_OBJECT
    Q_PROPERTY(QQuickWindow* window READ window WRITE setWindow NOTIFY windowChanged)
public:
    using QObject::QObject;
    QQuickWindow* window() const{return m_window;} void setWindow(QQuickWindow*);
signals: void windowChanged();
protected: bool eventFilter(QObject*,QEvent*) override;
private: QPointer<QQuickWindow> m_window;
};
