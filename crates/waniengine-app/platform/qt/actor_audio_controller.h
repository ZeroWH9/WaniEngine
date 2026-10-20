#pragma once
#include <QObject>
#include <QDBusContext>
#include <QJsonObject>
#include <QHash>
#include <QElapsedTimer>
class QMediaPlayer;class QAudioOutput;class QTimer;
class ActorAudioController : public QObject,protected QDBusContext {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface","org.waniengine.ActorAudio")
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
public:
    explicit ActorAudioController(QObject*parent=nullptr);~ActorAudioController() override;
    QString lastError() const{return m_error;}
    Q_INVOKABLE void configure(const QString&actors,const QString&audioOptions,const QString&monitor);
    Q_INVOKABLE void clear();
    Q_INVOKABLE void testSound(const QString&url);
    Q_INVOKABLE void testTrack(const QString&url,const QString&track);
public slots:
    Q_SCRIPTABLE void desktopClick(double x,double y);
signals: void lastErrorChanged(); void soundStarted(const QString&url,const QString&track);
private:
    struct Voice {QJsonObject actor,audio;QMediaPlayer*player=nullptr;QAudioOutput*output=nullptr;QTimer*timer=nullptr;int next=0;QElapsedTimer lastPlay;};
    void play(Voice *voice,bool forced=false,int requested=-1);void updateOutput(Voice*voice);void error(const QString&text);
    QHash<QString,Voice*> m_voices;QJsonObject m_options;QString m_monitor,m_error;bool m_registered=false;
};
