#pragma once
#include <QJsonObject>
#include <QObject>
#include <QVariantList>

class PlasmaIntegration;
class GnomeIntegration;

class DesktopIntegration final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool active READ isActive NOTIFY activeChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
    Q_PROPERTY(QVariantList monitors READ monitors NOTIFY monitorsChanged)
public:
    explicit DesktopIntegration(QObject *parent = nullptr);
    ~DesktopIntegration() override;

    bool isActive() const;
    QString lastError() const;
    QVariantList monitors() const;

    Q_INVOKABLE bool applyContent(const QString &selectedUrl, const QString &selectedKind,
                                  const QString &actorsJson, const QString &monitorName,
                                  bool audioEnabled, const QString &fitMode = QStringLiteral("cover"),
                                  int speed = 100, int volume = 50,
                                  bool playing = true, bool loop = true,
                                  const QString &visualOptions = QStringLiteral("{}"),
                                  const QString &audioOptions = QStringLiteral("{}"));
    Q_INVOKABLE bool updateAudioOptions(const QString &json);
    Q_INVOKABLE bool updateContentOptions(const QString &contentUrl, const QString &monitorName, const QString &json);
    Q_INVOKABLE bool restore();
    Q_INVOKABLE bool updateActors(const QString &actorsJson, const QString &monitorName);

    bool isKde() const { return m_isKde; }

signals:
    void activeChanged();
    void lastErrorChanged();
    void monitorsChanged();

private:
    bool m_isKde = false;
    PlasmaIntegration *m_plasma = nullptr;
    GnomeIntegration *m_gnome = nullptr;
};
