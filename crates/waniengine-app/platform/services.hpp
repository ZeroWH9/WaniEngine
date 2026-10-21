#pragma once
#include <QObject>
#include <QUrl>
#include <QVariantList>
#include <QVariantMap>
class QWindow;
class ApplicationController;

// Resolved in Rust so XDG_DOCUMENTS_DIR (localized by the desktop) is respected.
QString waniEngineLibraryDirectory();
// Copies projects from a previous library location without touching the origin.
int waniEngineMigrateLibrary(const QString &root, const QString &legacy);

// Thin Qt bindings; library and playlist storage are implemented in Rust.
class RustSceneStore final : public QObject {
    Q_OBJECT
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
public:
    RustSceneStore(QString root, QString source, QObject *parent=nullptr);
    QString lastError() const { return m_error; }
    Q_INVOKABLE QString loadScene();
    Q_INVOKABLE bool saveScene(const QString &json);
    Q_INVOKABLE QString organizeAsset(const QString &json);
    Q_INVOKABLE QString addAudioFiles(const QString &asset, const QString &sources);
    Q_INVOKABLE bool openFolder(const QString &url);
    Q_INVOKABLE QString libraryDirectory() const;
    Q_INVOKABLE QVariantList sampleLevels() const;
    Q_INVOKABLE QString describeMedia(const QString &source) const;
signals: void lastErrorChanged();
private: void error(const QString &message); QString m_root,m_source,m_error;
};

class RustPlaylistStore final : public QObject {
    Q_OBJECT
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
public:
    explicit RustPlaylistStore(QString root, QObject *parent=nullptr):QObject(parent),m_root(std::move(root)){}
    QString lastError() const { return m_error; }
    Q_INVOKABLE QUrl directory() const;
    Q_INVOKABLE QString load(const QUrl &url);
    Q_INVOKABLE bool save(const QUrl &url, const QString &json);
signals: void lastErrorChanged();
private: QString m_root,m_error;
};

class NativeFilePicker final : public QObject {
    Q_OBJECT
public:
    using QObject::QObject;
    void setWindow(QWindow *window) { m_window=window; }
    Q_INVOKABLE QVariantList openFiles(const QString &title,const QStringList &filters);
    Q_INVOKABLE QUrl openFile(const QString &title,const QString &folder,const QStringList &filters);
    Q_INVOKABLE QUrl saveFile(const QString &title,const QString &folder,const QStringList &filters);
private: QWindow *m_window=nullptr;
};

class RustApplicationController final : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool managerActive READ managerActive NOTIFY managerActiveChanged)
    Q_PROPERTY(bool trayAvailable READ trayAvailable NOTIFY trayAvailabilityChanged)
public:
    explicit RustApplicationController(ApplicationController *controller,QObject *parent=nullptr);
    bool managerActive() const;
    bool trayAvailable() const;
    Q_INVOKABLE void closeWindow(bool toTray);
    Q_INVOKABLE void requestExit();
    Q_INVOKABLE bool setStartWithSession(bool enabled);
signals: void managerActiveChanged(); void trayAvailabilityChanged();
private: ApplicationController *m_controller;
};

class VideoPreparation final : public QObject {
    Q_OBJECT
public:
    using QObject::QObject;
    Q_INVOKABLE QVariantMap prepareVideo(const QString &source,int width,int height,int monitorWidth,int monitorHeight) const;
    Q_INVOKABLE void prepareVideoAsync(const QString &source,int width,int height,int monitorWidth,int monitorHeight);
    Q_INVOKABLE void preparePreviewAsync(const QString &source,int width,int height,int monitorWidth,int monitorHeight,int request);
signals: void prepared(const QVariantMap &result); void previewPrepared(const QVariantMap &result);
private:
    void startPreview();
    QVariantMap m_pendingPreview;
    bool m_previewBusy=false;
};
