#include "application_controller.h"
#include "plasma_integration.h"
#include <QApplication>
#include <QDir>
#include <QFile>
#include <QDBusConnection>
#include <QDBusServiceWatcher>
#include <QDebug>
#include <QFileInfo>
#include <QSaveFile>
#include <QStandardPaths>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QSocketNotifier>
#include <QWindow>
#include <csignal>
#include <sys/socket.h>
#include <unistd.h>

using namespace Qt::StringLiterals;
namespace {
volatile sig_atomic_t signalWriteFd = -1;
void handleTermination(int signal)
{
    const unsigned char value = static_cast<unsigned char>(signal);
    if (signalWriteFd >= 0) { const auto ignored = ::write(signalWriteFd,&value,1); (void)ignored; }
}
}

ApplicationController::ApplicationController(PlasmaIntegration *integration, const QString &socketPath, QObject *parent)
    : QObject(parent), m_integration(integration), m_tray(QIcon(u":/icons/waniengine.png"_s)),
      m_watcher(new QDBusServiceWatcher(u"org.kde.StatusNotifierWatcher"_s,QDBusConnection::sessionBus(),
                                      QDBusServiceWatcher::WatchForOwnerChange,this))
{
    connect(qGuiApp,&QGuiApplication::applicationStateChanged,this,[this]{emit managerActiveChanged();});
    m_tray.setToolTip(u"WaniEngine"_s);
    m_menu.addAction(u"Open WaniEngine"_s,this,&ApplicationController::requestOpen)->setObjectName(u"trayOpen"_s);
    m_menu.addSeparator();
    m_menu.addAction(u"Exit"_s,this,&ApplicationController::requestExit)->setObjectName(u"trayExit"_s);
    m_tray.setContextMenu(&m_menu);
    connect(&m_tray,&QSystemTrayIcon::activated,this,[this](QSystemTrayIcon::ActivationReason reason) {
        if (reason == QSystemTrayIcon::Trigger || reason == QSystemTrayIcon::DoubleClick) requestOpen();
    });
    connect(m_watcher,&QDBusServiceWatcher::serviceOwnerChanged,this,[this] { updateTrayAvailability(); });
    // Qt re-registers a visible icon when the Plasma tray becomes available again.
    m_tray.show();
    updateTrayAvailability();
    QLocalServer::removeServer(socketPath); // The caller owns the single-instance lock.
    m_server.setSocketOptions(QLocalServer::UserAccessOption);
    if (!m_server.listen(socketPath)) qWarning("Could not start the local application activation socket.");
    connect(&m_server,&QLocalServer::newConnection,this,[this] {
        while (QLocalSocket *socket = m_server.nextPendingConnection()) {
            connect(socket,&QLocalSocket::disconnected,socket,&QObject::deleteLater);
            auto readCommand = [this,socket] {
                if (!socket->canReadLine()) return;
                const QJsonObject command = QJsonDocument::fromJson(socket->readLine(4096)).object();
                const QString action = command.value(u"action"_s).toString();
                if (action == u"open"_s) requestOpen();
                else if (action == u"exit"_s) requestExit();
                socket->disconnectFromServer();
            };
            connect(socket,&QLocalSocket::readyRead,this,readCommand);
            readCommand();
        }
    });
    if (::socketpair(AF_UNIX,SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC,0,m_signalSockets) == 0) {
        signalWriteFd = m_signalSockets[1];
        m_signalNotifier = new QSocketNotifier(m_signalSockets[0],QSocketNotifier::Read,this);
        connect(m_signalNotifier,&QSocketNotifier::activated,this,[this] {
            char bytes[32];
            const auto ignored = ::read(m_signalSockets[0],bytes,sizeof(bytes));
            (void)ignored;
            requestExit(); // All Qt/D-Bus work happens on the event loop, outside the POSIX handler.
        });
        std::signal(SIGTERM,handleTermination);
        std::signal(SIGINT,handleTermination);
    }
}
ApplicationController::~ApplicationController()
{
    signalWriteFd = -1;
    std::signal(SIGTERM,SIG_DFL);
    std::signal(SIGINT,SIG_DFL);
    if (m_signalNotifier) m_signalNotifier->setEnabled(false);
    for (int descriptor : m_signalSockets) if (descriptor >= 0) ::close(descriptor);
}
bool ApplicationController::managerActive() const {return qGuiApp->applicationState()==Qt::ApplicationActive;}
bool ApplicationController::trayAvailable() const { return m_trayAvailable; }
void ApplicationController::setWindow(QWindow *window) { m_window = window; }
void ApplicationController::updateTrayAvailability()
{
    const bool available = QSystemTrayIcon::isSystemTrayAvailable();
    if (available != m_trayAvailable) { m_trayAvailable = available; emit trayAvailabilityChanged(); }
}
void ApplicationController::requestOpen()
{
    if (!m_window) return;
    if (m_window->visibility()==QWindow::Minimized) m_window->showNormal(); else m_window->show();
    m_window->raise();
    m_window->requestActivate();
}
void ApplicationController::closeWindow(bool closeToTray)
{
    if (closeToTray && m_window) {
        if (m_trayAvailable) m_window->hide(); else m_window->showMinimized();
        return;
    }
    requestExit();
}
void ApplicationController::requestExit()
{
    if (!m_integration->restore())
        qWarning().noquote() << "The desktop recovery process will retry:" << m_integration->lastError();
    QCoreApplication::quit();
}
