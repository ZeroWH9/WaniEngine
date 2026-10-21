#pragma once
#include <QJsonObject>
#include <QObject>
#include <QProcess>
#include <QVariantList>
#include <QMap>

class GnomeIntegration final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool active READ isActive NOTIFY activeChanged)
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
    Q_PROPERTY(QVariantList monitors READ monitors NOTIFY monitorsChanged)
public:
    explicit GnomeIntegration(QObject *parent = nullptr);
    ~GnomeIntegration() override;

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

    static int runWallpaperWorker(int argc, char **argv);

signals:
    void activeChanged();
    void lastErrorChanged();
    void monitorsChanged();

private:
    struct WorkerState {
        QProcess *process = nullptr;
        QJsonObject parameters;
    };

    void setError(const QString &message);
    void backupStaticWallpaper();
    void restoreStaticWallpaper();
    void saveRecovery() const;
    bool loadRecovery();

    QMap<QString, WorkerState> m_workers;
    bool m_active = false;
    QString m_lastError;
    QString m_sessionPath;
    QString m_originalPictureUri;
    QString m_originalPictureUriDark;
    QString m_appliedPictureUri;
    QString m_appliedPictureUriDark;
};
