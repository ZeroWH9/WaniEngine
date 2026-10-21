#include "desktop_integration.h"
#include "plasma_integration.h"
#include "gnome_integration.h"
#include <QCoreApplication>

using namespace Qt::StringLiterals;

namespace {
// GNOME-family sessions run the wallpaper through an X11/XWayland desktop
// layer. Anything else keeps the Plasma backend so existing behavior is
// preserved when the session cannot be identified.
bool isGnomeSession()
{
    const QString desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP").toUpper();
    if (desktop.contains(u"GNOME"_s, Qt::CaseInsensitive)
        || desktop.contains(u"UNITY"_s, Qt::CaseInsensitive)
        || desktop.contains(u"BUDGIE"_s, Qt::CaseInsensitive)
        || desktop.contains(u"PANTHEON"_s, Qt::CaseInsensitive)
        || desktop.contains(u"CINNAMON"_s, Qt::CaseInsensitive)) {
        return true;
    }
    return qEnvironmentVariable("GNOME_DESKTOP_SESSION_ID").size() > 0;
}

bool isPlasmaSession()
{
    const QString desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP").toUpper();
    return desktop.contains(u"KDE"_s, Qt::CaseInsensitive)
        || desktop.contains(u"PLASMA"_s, Qt::CaseInsensitive)
        || !qEnvironmentVariable("KDE_FULL_SESSION").isEmpty();
}
}

DesktopIntegration::DesktopIntegration(QObject *parent)
    : QObject(parent),
      m_isKde(!isGnomeSession() || isPlasmaSession())
{
    if (m_isKde) {
        m_plasma = new PlasmaIntegration(this);
        connect(m_plasma, &PlasmaIntegration::activeChanged, this, &DesktopIntegration::activeChanged);
        connect(m_plasma, &PlasmaIntegration::lastErrorChanged, this, &DesktopIntegration::lastErrorChanged);
        connect(m_plasma, &PlasmaIntegration::monitorsChanged, this, &DesktopIntegration::monitorsChanged);
    } else {
        m_gnome = new GnomeIntegration(this);
        connect(m_gnome, &GnomeIntegration::activeChanged, this, &DesktopIntegration::activeChanged);
        connect(m_gnome, &GnomeIntegration::lastErrorChanged, this, &DesktopIntegration::lastErrorChanged);
        connect(m_gnome, &GnomeIntegration::monitorsChanged, this, &DesktopIntegration::monitorsChanged);
    }
}

DesktopIntegration::~DesktopIntegration()
{
    restore();
}

bool DesktopIntegration::isActive() const
{
    return m_plasma ? m_plasma->isActive() : (m_gnome ? m_gnome->isActive() : false);
}

QString DesktopIntegration::lastError() const
{
    return m_plasma ? m_plasma->lastError() : (m_gnome ? m_gnome->lastError() : QString());
}

QVariantList DesktopIntegration::monitors() const
{
    return m_plasma ? m_plasma->monitors() : (m_gnome ? m_gnome->monitors() : QVariantList());
}

bool DesktopIntegration::applyContent(const QString &selectedUrl, const QString &selectedKind,
                                      const QString &actorsJson, const QString &monitorName,
                                      bool audioEnabled, const QString &fitMode,
                                      int speed, int volume,
                                      bool playing, bool loop,
                                      const QString &visualOptions, const QString &audioOptions)
{
    if (m_plasma) {
        return m_plasma->applyContent(selectedUrl, selectedKind, actorsJson, monitorName,
                                      audioEnabled, fitMode, speed, volume, playing, loop,
                                      visualOptions, audioOptions);
    }
    if (m_gnome) {
        return m_gnome->applyContent(selectedUrl, selectedKind, actorsJson, monitorName,
                                     audioEnabled, fitMode, speed, volume, playing, loop,
                                     visualOptions, audioOptions);
    }
    return false;
}

bool DesktopIntegration::updateAudioOptions(const QString &json)
{
    if (m_plasma) return m_plasma->updateAudioOptions(json);
    if (m_gnome) return m_gnome->updateAudioOptions(json);
    return false;
}

bool DesktopIntegration::updateContentOptions(const QString &contentUrl, const QString &monitorName, const QString &json)
{
    if (m_plasma) return m_plasma->updateContentOptions(contentUrl, monitorName, json);
    if (m_gnome) return m_gnome->updateContentOptions(contentUrl, monitorName, json);
    return false;
}

bool DesktopIntegration::restore()
{
    if (m_plasma) return m_plasma->restore();
    if (m_gnome) return m_gnome->restore();
    return true;
}

bool DesktopIntegration::updateActors(const QString &actorsJson, const QString &monitorName)
{
    if (m_plasma) return m_plasma->updateActors(actorsJson, monitorName);
    if (m_gnome) return m_gnome->updateActors(actorsJson, monitorName);
    return true;
}
