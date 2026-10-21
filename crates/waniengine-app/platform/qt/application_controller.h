#pragma once
#include <QObject>
#include <QLocalServer>
#include <QMenu>
#include <QSystemTrayIcon>
#include <QString>

class DesktopIntegration;
class QWindow;
class QDBusServiceWatcher;
class QSocketNotifier;

class ApplicationController final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool managerActive READ managerActive NOTIFY managerActiveChanged)
    Q_PROPERTY(bool trayAvailable READ trayAvailable NOTIFY trayAvailabilityChanged)
public:
    explicit ApplicationController(DesktopIntegration *integration, const QString &socketPath, QObject *parent = nullptr);
    ~ApplicationController() override;
    bool trayAvailable() const;
    bool managerActive() const;
    void setWindow(QWindow *window);
    Q_INVOKABLE void requestOpen();
    Q_INVOKABLE void closeWindow(bool closeToTray = true);
    Q_INVOKABLE void requestExit();
signals:
    void trayAvailabilityChanged();
    void managerActiveChanged();
private:
    void updateTrayAvailability();
    DesktopIntegration *m_integration;
    QWindow *m_window = nullptr;
    QString m_socketPath;
    QSystemTrayIcon m_tray;
    QMenu m_menu;
    QLocalServer m_server;
    QDBusServiceWatcher *m_watcher;
    bool m_trayAvailable = false;
    int m_signalSockets[2] = {-1,-1};
    QSocketNotifier *m_signalNotifier = nullptr;
};
