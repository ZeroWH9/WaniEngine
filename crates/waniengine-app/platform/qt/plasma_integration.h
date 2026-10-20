#pragma once
#include <QJsonObject>
#include <QObject>
#include <QVariantList>

class PlasmaIntegration final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool active READ isActive NOTIFY activeChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
    Q_PROPERTY(QVariantList monitors READ monitors NOTIFY monitorsChanged)
public:
    enum class Mode { Manager, Recovery };
    explicit PlasmaIntegration(QObject *parent = nullptr, Mode mode = Mode::Manager);
    ~PlasmaIntegration() override;
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
    static int runGuardian(int argc, char **argv);
    static QString processIdentity(qint64 pid);
signals:
    void activeChanged();
    void lastErrorChanged();
    void monitorsChanged();
private:
    struct WallpaperSnapshot {
        int screen = 0;
        int containmentId = -1;
        QString originalPlugin;
        QString appliedPlugin;
        QString sessionToken;
    };
    QJsonObject evaluate(const QString &script);
    QString ensurePluginInstalled();
    bool ensureGuardian();
    bool loadRecovery();
    bool saveRecovery() const;
    void setError(const QString &message);
    int screenIndex(const QString &monitorName);
    QString m_sessionPath;
    QList<WallpaperSnapshot> m_snapshots;
    qint64 m_ownerPid = 0;
    QString m_ownerIdentity;
    qint64 m_guardianPid = 0;
    QString m_guardianIdentity;
    bool m_active = false;
    QString m_lastError;
    QString m_validatedPackage;
    Mode m_mode;
};
