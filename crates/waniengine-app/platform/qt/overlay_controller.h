#pragma once
#include <QObject>
#include <QProcess>

// Characters have their own Wayland client; they never alter Plasma's wallpaper.
class OverlayController final : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool active READ active NOTIFY activeChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
public:
    explicit OverlayController(QObject *parent = nullptr);
    ~OverlayController() override;
    bool active() const;
    QString lastError() const { return m_error; }
    qint64 workerPid() const { return m_worker.processId(); }
    Q_INVOKABLE bool applyActors(const QString &json, const QString &monitor);
    Q_INVOKABLE void clear();
    static int runWorker(int argc, char **argv);
signals:
    void activeChanged();
    void lastErrorChanged();
private:
    void error(const QString &message);
    QProcess m_worker;
    QString m_monitor, m_error;
};
