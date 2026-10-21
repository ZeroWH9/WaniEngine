#include "services.hpp"
#include "waniengine/src/bridge.cxx.h"
#include "media_metadata.h"
#include "application_controller.h"
#include "media_optimizer.h"
#include <QtConcurrentRun>
#include <QFutureWatcher>
#include <QCoreApplication>
#include <QDesktopServices>
#include <QDir>
#include <QFileDialog>
#include <QJsonDocument>
#include <QOpenGLContext>
#include <QOpenGLExtraFunctions>
#include <QOpenGLFramebufferObject>
#include <QOffscreenSurface>
#include <QStandardPaths>
#include <QWindow>
using namespace Qt::StringLiterals;
namespace {
QString qtString(const rust::String &string) { return QString::fromUtf8(string.data(),static_cast<qsizetype>(string.size())); }
rust::Str rustString(const QByteArray &string) { return {string.constData(),static_cast<size_t>(string.size())}; }
}
QString waniEngineLibraryDirectory() { try {return qtString(waniengine::library_directory());}catch(const std::exception &e){qWarning("Library directory: %s",e.what());return QDir::homePath()+u"/WaniEngine"_s;} }

int waniEngineMigrateLibrary(const QString &root,const QString &legacy) { try {return static_cast<int>(waniengine::migrate_library(rustString(root.toUtf8()),rustString(legacy.toUtf8())));}catch(const std::exception &e){qWarning("Library migration: %s",e.what());return 0;} }

RustSceneStore::RustSceneStore(QString root,QString source,QObject *parent):QObject(parent),m_root(std::move(root)),m_source(std::move(source)){}
void RustSceneStore::error(const QString &message) { if(m_error==message)return;m_error=message;emit lastErrorChanged(); }
QString RustSceneStore::loadScene() { try {auto result=waniengine::load_scene(rustString(m_root.toUtf8()),rustString(m_source.toUtf8()));error({});return qtString(result);}catch(const std::exception &e){error(QString::fromUtf8(e.what()));return u"{}"_s;} }
bool RustSceneStore::saveScene(const QString &json) { try {waniengine::save_scene(rustString(m_root.toUtf8()),rustString(json.toUtf8()));error({});return true;}catch(const std::exception &e){error(QString::fromUtf8(e.what()));return false;} }
QString RustSceneStore::organizeAsset(const QString &json) { try {auto result=waniengine::organize_asset(rustString(m_root.toUtf8()),rustString(json.toUtf8()));error({});return qtString(result);}catch(const std::exception &e){error(QString::fromUtf8(e.what()));return u"{}"_s;} }
QString RustSceneStore::addAudioFiles(const QString &asset,const QString &sources) { try {auto result=waniengine::add_audio(rustString(m_root.toUtf8()),rustString(asset.toUtf8()),rustString(sources.toUtf8()));error({});return qtString(result);}catch(const std::exception &e){error(QString::fromUtf8(e.what()));return u"[]"_s;} }
bool RustSceneStore::openFolder(const QString &url) {return QDesktopServices::openUrl(QUrl(url));}
QString RustSceneStore::libraryDirectory() const {const auto videos=QStandardPaths::writableLocation(QStandardPaths::MoviesLocation);const auto folder=QDir(videos.isEmpty()?QDir::homePath()+u"/Videos"_s:videos).filePath(u"WaniEngine"_s);QDir().mkpath(folder);return folder;}
QString RustSceneStore::describeMedia(const QString &source) const {return QString::fromUtf8(QJsonDocument(describeLocalMedia(QUrl(source),QDir(m_root).filePath(u"thumbnails"_s))).toJson(QJsonDocument::Compact));}
QVariantList RustSceneStore::sampleLevels() const {
    QOpenGLContext context;QSurfaceFormat format;format.setRenderableType(QSurfaceFormat::OpenGL);format.setVersion(3,3);context.setFormat(format);if(!context.create())return {0};
    QOffscreenSurface surface;surface.setFormat(context.format());surface.create();if(!context.makeCurrent(&surface))return {0};
    GLint maximum=0;context.extraFunctions()->glGetIntegerv(GL_MAX_SAMPLES,&maximum);QVariantList levels{0};
    for(int samples:{2,4,8,16})if(samples<=maximum){QOpenGLFramebufferObjectFormat f;f.setSamples(samples);f.setInternalTextureFormat(GL_RGBA8);QOpenGLFramebufferObject buffer(QSize(16,16),f);if(buffer.isValid()&&buffer.format().samples()==samples)levels.append(samples);}
    context.doneCurrent();return levels;
}
QUrl RustPlaylistStore::directory() const {const auto folder=QDir(m_root).filePath(u"playlists"_s);QDir().mkpath(folder);return QUrl::fromLocalFile(folder);}
QString RustPlaylistStore::load(const QUrl &url) {try {auto result=waniengine::load_playlist(rustString(url.toString().toUtf8()));m_error.clear();return qtString(result);}catch(const std::exception &e){m_error=QString::fromUtf8(e.what());emit lastErrorChanged();return {};}}
bool RustPlaylistStore::save(const QUrl &url,const QString &json) {try {waniengine::save_playlist(rustString(url.toString().toUtf8()),rustString(json.toUtf8()));m_error.clear();return true;}catch(const std::exception &e){m_error=QString::fromUtf8(e.what());emit lastErrorChanged();return false;}}
namespace {
void configurePicker(QFileDialog &dialog,QWindow *window,const QString &title,const QStringList &filters) {
    dialog.setWindowTitle(title);dialog.setNameFilters(filters);dialog.setOption(QFileDialog::DontUseNativeDialog,false);
    dialog.setModal(true);dialog.winId();if(window&&dialog.windowHandle())dialog.windowHandle()->setTransientParent(window);
}
}
QVariantList NativeFilePicker::openFiles(const QString &title,const QStringList &filters) {QFileDialog dialog;configurePicker(dialog,m_window,title,filters);dialog.setFileMode(QFileDialog::ExistingFiles);dialog.setAcceptMode(QFileDialog::AcceptOpen);QVariantList files;if(dialog.exec()==QDialog::Accepted)for(const auto &url:dialog.selectedUrls())files.append(url);return files;}
QUrl NativeFilePicker::openFile(const QString &title,const QString &folder,const QStringList &filters) {QFileDialog dialog;configurePicker(dialog,m_window,title,filters);dialog.setDirectoryUrl(QUrl(folder));dialog.setFileMode(QFileDialog::ExistingFile);return dialog.exec()==QDialog::Accepted&&!dialog.selectedUrls().isEmpty()?dialog.selectedUrls().first():QUrl();}
QUrl NativeFilePicker::saveFile(const QString &title,const QString &folder,const QStringList &filters) {QFileDialog dialog;configurePicker(dialog,m_window,title,filters);dialog.setDirectoryUrl(QUrl(folder));dialog.setAcceptMode(QFileDialog::AcceptSave);dialog.setDefaultSuffix(u"json"_s);return dialog.exec()==QDialog::Accepted&&!dialog.selectedUrls().isEmpty()?dialog.selectedUrls().first():QUrl();}
RustApplicationController::RustApplicationController(ApplicationController *controller,QObject *parent):QObject(parent),m_controller(controller){connect(controller,&ApplicationController::managerActiveChanged,this,&RustApplicationController::managerActiveChanged);connect(controller,&ApplicationController::trayAvailabilityChanged,this,&RustApplicationController::trayAvailabilityChanged);}
bool RustApplicationController::managerActive() const{return m_controller->managerActive();}
bool RustApplicationController::trayAvailable() const{return m_controller->trayAvailable();}
void RustApplicationController::closeWindow(bool toTray){m_controller->closeWindow(toTray);}
void RustApplicationController::requestExit(){m_controller->requestExit();}
bool RustApplicationController::setStartWithSession(bool enabled){if(QCoreApplication::applicationName().startsWith(u"waniengine-inspection-"_s))return true;try{waniengine::set_autostart(enabled,rustString(QCoreApplication::applicationFilePath().toUtf8()));return true;}catch(const std::exception &e){qWarning("Autostart: %s",e.what());return false;}}
QVariantMap VideoPreparation::prepareVideo(const QString &source,int width,int height,int monitorWidth,int monitorHeight) const {MediaOptimizer optimizer;return optimizer.prepareVideo(source,width,height,monitorWidth,monitorHeight);}
void VideoPreparation::prepareVideoAsync(const QString &source,int width,int height,int monitorWidth,int monitorHeight) {
    auto *watcher=new QFutureWatcher<QVariantMap>(this);
    connect(watcher,&QFutureWatcher<QVariantMap>::finished,this,[this,watcher]{emit prepared(watcher->result());watcher->deleteLater();});
    watcher->setFuture(QtConcurrent::run([source,width,height,monitorWidth,monitorHeight]{MediaOptimizer optimizer;return optimizer.prepareVideo(source,width,height,monitorWidth,monitorHeight);}));
}
void VideoPreparation::preparePreviewAsync(const QString &source,int width,int height,int monitorWidth,int monitorHeight,int request) {
    // Only one preview job and the newest queued request are retained.
    m_pendingPreview={{u"source"_s,source},{u"width"_s,width},{u"height"_s,height},{u"targetWidth"_s,monitorWidth},{u"targetHeight"_s,monitorHeight},{u"request"_s,request}};
    if(!m_previewBusy)startPreview();
}
void VideoPreparation::startPreview() {
    if(m_pendingPreview.isEmpty())return;
    const auto request=m_pendingPreview;m_pendingPreview.clear();m_previewBusy=true;
    auto *watcher=new QFutureWatcher<QVariantMap>(this);
    connect(watcher,&QFutureWatcher<QVariantMap>::finished,this,[this,watcher,request]{auto result=watcher->result();result.insert(u"source"_s,request.value(u"source"_s));result.insert(u"request"_s,request.value(u"request"_s));watcher->deleteLater();m_previewBusy=false;emit previewPrepared(result);startPreview();});
    watcher->setFuture(QtConcurrent::run([request]{MediaOptimizer optimizer;return optimizer.prepareVideo(request.value(u"source"_s).toString(),request.value(u"width"_s).toInt(),request.value(u"height"_s).toInt(),request.value(u"targetWidth"_s).toInt(),request.value(u"targetHeight"_s).toInt());}));
}
