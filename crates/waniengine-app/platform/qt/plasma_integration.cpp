#include "runtime_paths.h"
#include "plasma_integration.h"
#include "plasma_scripts.h"

#include <QColor>
#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusReply>
#include <QDir>
#include <QDirIterator>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QProcess>
#include <QProcessEnvironment>
#include <QSaveFile>
#include <QScreen>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QTimer>
#include <QUuid>
#include <QUrl>
#include <algorithm>
#include <iterator>
#include <utility>

using namespace Qt::StringLiterals;

namespace {
constexpr auto kPluginPrefix = "org.waniengine.wallpaper.v";
bool localMedia(const QString &url)
{
    const QUrl parsed(url);
    return parsed.isLocalFile() && QFileInfo(parsed.toLocalFile()).isFile();
}
QString contentKind(const QString &url)
{
    const QString path = QUrl(url).toLocalFile();
    if (QStringList{u"mp4"_s,u"mkv"_s,u"webm"_s,u"mov"_s,u"avi"_s,u"m4v"_s}.contains(QFileInfo(path).suffix().toLower()))
        return u"video"_s;
    QImageReader reader(path);
    return reader.supportsAnimation() ? u"animated"_s : u"image"_s;
}
QString normalizedColor(const QString &value)
{
    const QColor parsed(value);
    if (parsed.isValid()) return parsed.name(QColor::HexArgb);
    const QStringList components = value.split(',');
    if (components.size() >= 3)
        return QColor(components[0].toInt(),components[1].toInt(),components[2].toInt()).name();
    return u"#202020"_s;
}
}

PlasmaIntegration::PlasmaIntegration(QObject *parent, Mode mode)
    : QObject(parent), m_sessionPath(QStandardPaths::writableLocation(QStandardPaths::StateLocation) + u"/plasma-session.json"_s), m_mode(mode)
{
    if (auto *gui = qobject_cast<QGuiApplication *>(QCoreApplication::instance())) {
        auto trackScreen=[this](QScreen*screen){connect(screen,&QScreen::refreshRateChanged,this,&PlasmaIntegration::monitorsChanged);connect(screen,&QScreen::geometryChanged,this,&PlasmaIntegration::monitorsChanged);};
        for(auto*screen:gui->screens())trackScreen(screen);
        connect(gui,&QGuiApplication::screenAdded,this,[this,trackScreen](QScreen*screen){trackScreen(screen);emit monitorsChanged();});
        connect(gui,&QGuiApplication::screenRemoved,this,&PlasmaIntegration::monitorsChanged);
    }
    if (loadRecovery() && mode == Mode::Manager && m_ownerPid != QCoreApplication::applicationPid()
        && (processIdentity(m_ownerPid).isEmpty()
            || (!m_ownerIdentity.isEmpty() && processIdentity(m_ownerPid) != m_ownerIdentity)))
        restore();
}
PlasmaIntegration::~PlasmaIntegration() { if (m_mode == Mode::Manager) restore(); }
bool PlasmaIntegration::isActive() const { return m_active; }
QString PlasmaIntegration::lastError() const { return m_lastError; }

QVariantList PlasmaIntegration::monitors() const
{
    QVariantList result;
    if (!qobject_cast<QGuiApplication *>(QCoreApplication::instance())) return result;
    for (QScreen *screen : QGuiApplication::screens())
        result.push_back(QVariantMap{{u"name"_s,screen->name()},{u"width"_s,screen->size().width()},{u"height"_s,screen->size().height()},{u"pixelWidth"_s,qRound(screen->size().width()*screen->devicePixelRatio())},{u"pixelHeight"_s,qRound(screen->size().height()*screen->devicePixelRatio())},{u"refreshRate"_s,screen->refreshRate()},{u"scale"_s,screen->devicePixelRatio()}});
    return result;
}
int PlasmaIntegration::screenIndex(const QString &name)
{
    const auto screens = QGuiApplication::screens();
    for (qsizetype i = 0; i < screens.size(); ++i)
        if (screens[i]->name() == name) {
            const auto result = evaluate(PlasmaScripts::resolveScreen(screens[i]->geometry()));
            return result.value(u"ok"_s).toBool() ? result.value(u"screen"_s).toInt(-1) : -1;
        }
    return -1;
}
QString PlasmaIntegration::processIdentity(qint64 pid)
{
    if (pid <= 0) return {};
    QFile file(u"/proc/%1/stat"_s.arg(pid));
    if (!file.open(QIODevice::ReadOnly)) return {};
    const QByteArray stat = file.readAll();
    const QList<QByteArray> fields = stat.mid(stat.lastIndexOf(')')+2).split(' ');
    return QString::fromLatin1(fields.value(19)); // Linux process start-time, unaffected by PID reuse.
}

QJsonObject PlasmaIntegration::evaluate(const QString &script)
{
    auto message = QDBusMessage::createMethodCall(u"org.kde.plasmashell"_s,u"/PlasmaShell"_s,u"org.kde.PlasmaShell"_s,u"evaluateScript"_s);
    message << script;
    const QDBusReply<QString> reply = QDBusConnection::sessionBus().call(message,QDBus::Block,3000);
    if (!reply.isValid()) {
        const QString error = u"Could not communicate with Plasma: %1"_s.arg(reply.error().message());
        setError(error);
        return {{u"ok"_s,false},{u"error"_s,error}};
    }
    const QJsonObject result = PlasmaScripts::parseResult(reply.value());
    if (!result.value(u"ok"_s).toBool()) setError(result.value(u"error"_s).toString());
    return result;
}

QString PlasmaIntegration::ensurePluginInstalled()
{
#ifdef WANIENGINE_PLASMA_PACKAGE_SOURCE
    const QString source = QDir(waniEngineWallpaperSource()).absolutePath();
#else
    const QString source;
#endif
    if (!QFileInfo::exists(QDir(source).filePath(u"metadata.json"_s))) {
        setError(u"The WaniEngine wallpaper package was not found."_s);
        return {};
    }
    QStringList relativeFiles;
    QDirIterator iterator(source,QDir::Files,QDirIterator::Subdirectories);
    while (iterator.hasNext()) relativeFiles.push_back(QDir(source).relativeFilePath(iterator.next()));
    relativeFiles.sort();
    QCryptographicHash hash(QCryptographicHash::Sha256);
    for (const QString &relative : std::as_const(relativeFiles)) {
        QFile file(QDir(source).filePath(relative));
        if (!file.open(QIODevice::ReadOnly)) { setError(u"Could not read the wallpaper package."_s); return {}; }
        hash.addData(relative.toUtf8());
        hash.addData(file.readAll());
    }
    const QString digest = QString::fromLatin1(hash.result().toHex().left(12));
    const QString plugin = QString::fromLatin1(kPluginPrefix) + digest;
    if (m_validatedPackage != plugin) {
        QProcess validator;
        QProcessEnvironment environment = QProcessEnvironment::systemEnvironment();
        environment.insert(u"QT_QPA_PLATFORM"_s,u"offscreen"_s);
        environment.insert(u"QT_QUICK_BACKEND"_s,u"software"_s);
        environment.insert(u"QT_LOGGING_TO_CONSOLE"_s,u"1"_s);
        environment.insert(u"KDE_DEBUG"_s,u"1"_s);
        validator.setProcessEnvironment(environment);
        validator.start(QCoreApplication::applicationFilePath(),{u"--validate-wallpaper"_s,source});
        if (!validator.waitForStarted(3000) || !validator.waitForFinished(10000)
            || validator.exitStatus() != QProcess::NormalExit || validator.exitCode() != 0) {
            validator.kill();
            validator.waitForFinished(1000);
            setError(u"The wallpaper package failed validation. Your current wallpaper was preserved. %1"_s
                         .arg(QString::fromUtf8(validator.readAllStandardError()).left(600)));
            return {};
        }
        m_validatedPackage = plugin;
    }
    const QString packageRoot = QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation)+u"/plasma/wallpapers"_s;
    const QString target = QDir(packageRoot).filePath(plugin);
    if (QFileInfo::exists(QDir(target).filePath(u"metadata.json"_s))) return plugin;
    if (!QDir().mkpath(packageRoot)) { setError(u"Could not create the Plasma wallpapers folder."_s); return {}; }
    QTemporaryDir staging(QDir(packageRoot).filePath(u".waniengine-XXXXXX"_s));
    if (!staging.isValid()) { setError(u"Could not prepare the wallpaper plugin."_s); return {}; }
    for (const QString &relative : std::as_const(relativeFiles)) {
        const QString destination = QDir(staging.path()).filePath(relative);
        if (!QDir().mkpath(QFileInfo(destination).absolutePath()) || !QFile::copy(QDir(source).filePath(relative),destination)) {
            setError(u"Could not install the wallpaper plugin files."_s); return {};
        }
    }
    // Every immutable wallpaper build has its own QML namespace, so Qt can keep
    // older cached packages alive while the new package is installed.
    QSaveFile module(QDir(staging.path()).filePath(u"contents/ui/runtime/qmldir"_s));
    if(!module.open(QIODevice::WriteOnly)||module.write((u"module %1.Runtime\nplugin waniengine_runtime\n"_s.arg(plugin)).toUtf8())<0||!module.commit()){
        setError(u"Could not install the media module."_s);return {};
    }
    const QString metadataPath = QDir(staging.path()).filePath(u"metadata.json"_s);
    QFile metadata(metadataPath);
    if (!metadata.open(QIODevice::ReadOnly)) return {};
    QJsonObject object = QJsonDocument::fromJson(metadata.readAll()).object();
    metadata.close();
    QJsonObject details = object.value(u"KPlugin"_s).toObject();
    details.insert(u"Id"_s,plugin);
    object.insert(u"KPlugin"_s,details);
    QSaveFile updated(metadataPath);
    if (!updated.open(QIODevice::WriteOnly) || updated.write(QJsonDocument(object).toJson()) < 0 || !updated.commit()
        || !QDir().rename(staging.path(),target)) {
        setError(u"Could not complete the wallpaper plugin installation."_s); return {};
    }
    return plugin;
}

bool PlasmaIntegration::ensureGuardian()
{
    if (m_guardianPid > 0 && !m_guardianIdentity.isEmpty() && processIdentity(m_guardianPid) == m_guardianIdentity) return true;
    QProcess guardian;
    guardian.setProgram(QCoreApplication::applicationFilePath());
    guardian.setArguments({u"--guardian"_s,QString::number(QCoreApplication::applicationPid()),
                           processIdentity(QCoreApplication::applicationPid()),m_sessionPath});
    guardian.setStandardOutputFile(QProcess::nullDevice());
    guardian.setStandardErrorFile(QProcess::nullDevice());
    if (!guardian.startDetached(&m_guardianPid)) {
        setError(u"Could not start desktop recovery. Your wallpaper was preserved."_s);
        return false;
    }
    m_guardianIdentity = processIdentity(m_guardianPid);
    return true;
}

bool PlasmaIntegration::applyContent(const QString &selectedUrl, const QString &selectedKind,
                                     const QString &actorsJson, const QString &monitorName,
                                     bool audioEnabled, const QString &fitMode, int speed, int volume,
                                     bool playing, bool loop, const QString &visualOptions, const QString &audioOptions)
{
    if (m_mode != Mode::Manager || !localMedia(selectedUrl)
        || (selectedKind != u"wallpaper"_s && selectedKind != u"character"_s)) {
        setError(u"Select a valid local image, animation or video."_s); return false;
    }
    const auto visualDocument=QJsonDocument::fromJson(visualOptions.toUtf8());
    if (!visualDocument.isObject()) { setError(u"Invalid visual options."_s); return false; }
    const auto audioDocument=QJsonDocument::fromJson(audioOptions.toUtf8());
    if (!audioDocument.isObject()) { setError(u"Invalid audio options."_s); return false; }
    const int screen = screenIndex(monitorName);
    if (screen < 0) { if (m_lastError.isEmpty()) setError(u"The selected monitor is disconnected."_s); return false; }
    const QJsonDocument actorDocument = QJsonDocument::fromJson(actorsJson.toUtf8());
    if (!actorDocument.isArray() || actorDocument.array().size() > 64) {
        setError(u"The character list is invalid or exceeds 64 entries."_s); return false;
    }
    for (const QJsonValue &actor : actorDocument.array()) {
        if (!actor.isObject() || !localMedia(actor.toObject().value(u"url"_s).toString())) {
            setError(u"A character file is unavailable."_s); return false;
        }
    }
    const QJsonObject before = evaluate(PlasmaScripts::inspect(screen));
    if (!before.value(u"ok"_s).toBool()) return false;
    const QString plugin = ensurePluginInstalled();
    if (plugin.isEmpty()) return false;
    const QString currentPlugin = before.value(u"plugin"_s).toString();
    QString url = selectedKind == u"wallpaper"_s ? selectedUrl : before.value(u"image"_s).toString();
    QString kind = !url.isEmpty() && localMedia(url) ? contentKind(url) : u"color"_s;
    QString fit = fitMode;
    int playbackSpeed = speed;
    int playbackVolume = volume;
    if (selectedKind == u"character"_s && currentPlugin.startsWith(QString::fromLatin1(kPluginPrefix))) {
        url = before.value(u"contentUrl"_s).toString();
        kind = before.value(u"contentKind"_s).toString();
        fit = before.value(u"fitMode"_s).toString();
        playbackSpeed = before.value(u"speed"_s).toInt(100);
        playbackVolume = before.value(u"volume"_s).toInt(50);
        audioEnabled = before.value(u"audioEnabled"_s).toBool();
    }
    if (url == u"none"_s) url.clear();
    if (!url.isEmpty() && !localMedia(url)) {
        setError(u"Apply a local wallpaper before adding characters to it."_s); return false;
    }
    if (!QStringList{u"cover"_s,u"contain"_s,u"stretch"_s,u"center"_s}.contains(fit)) fit = u"cover"_s;
    const auto previousSnapshots = m_snapshots;
    auto existing = std::find_if(m_snapshots.begin(),m_snapshots.end(),[screen](const WallpaperSnapshot &s){return s.screen==screen;});
    if (existing != m_snapshots.end() && (existing->appliedPlugin != currentPlugin
        || existing->sessionToken != before.value(u"sessionToken"_s).toString())) {
        m_snapshots.erase(existing);
        existing = m_snapshots.end();
    }
    if (existing == m_snapshots.end()) {
        m_snapshots.push_back({screen,before.value(u"id"_s).toInt(-1),currentPlugin,plugin,{}});
        existing = std::prev(m_snapshots.end());
    }
    existing->appliedPlugin = plugin;
    existing->sessionToken = QUuid::createUuid().toString(QUuid::WithoutBraces);
    const QString token = existing->sessionToken;
    const int id = existing->containmentId;
    m_ownerPid = QCoreApplication::applicationPid();
    m_ownerIdentity = processIdentity(m_ownerPid);
    if (!saveRecovery() || !ensureGuardian()) {
        m_snapshots = previousSnapshots;
        if (m_snapshots.isEmpty()) QFile::remove(m_sessionPath); else saveRecovery();
        if (m_lastError.isEmpty()) setError(u"Could not save desktop recovery data."_s);
        return false;
    }
    const QJsonObject parameters{{u"ContentUrl"_s,url.isEmpty()?u"none"_s:url},{u"ContentKind"_s,kind},
        {u"FitMode"_s,fit},{u"AudioEnabled"_s,audioEnabled},{u"BackgroundColor"_s,normalizedColor(before.value(u"color"_s).toString())},
        {u"ActorsJson"_s,QString::fromUtf8(actorDocument.toJson(QJsonDocument::Compact))},
        {u"Speed"_s,std::clamp(playbackSpeed,10,300)},{u"Volume"_s,std::clamp(playbackVolume,0,100)},
        {u"SessionToken"_s,token},{u"Playing"_s,playing},{u"Loop"_s,loop},
        {u"VisualOptionsJson"_s,QString::fromUtf8(visualDocument.toJson(QJsonDocument::Compact))},
        {u"AudioOptionsJson"_s,QString::fromUtf8(audioDocument.toJson(QJsonDocument::Compact))}};
    const QJsonObject applied = evaluate(PlasmaScripts::apply(id,currentPlugin,plugin,parameters));
    const QJsonObject after = evaluate(PlasmaScripts::inspect(screen,id));
    if (!applied.value(u"ok"_s).toBool() || !after.value(u"ok"_s).toBool()
        || after.value(u"plugin"_s).toString()!=plugin || after.value(u"sessionToken"_s).toString()!=token
        || after.value(u"contentUrl"_s).toString()!=parameters.value(u"ContentUrl"_s).toString()) {
        const QString error = m_lastError.isEmpty()?u"Plasma did not confirm the wallpaper change."_s:m_lastError;
        if (after.value(u"ok"_s).toBool() && after.value(u"plugin"_s).toString()==currentPlugin
            && after.value(u"sessionToken"_s)==before.value(u"sessionToken"_s)) {
            m_snapshots = previousSnapshots;
            if (m_snapshots.isEmpty()) QFile::remove(m_sessionPath); else saveRecovery();
        } else restore();
        setError(error); return false;
    }
    m_active = true;
    emit activeChanged();
    setError({});
    return true;
}

bool PlasmaIntegration::restore()
{
    QList<WallpaperSnapshot> remaining;
    for (const WallpaperSnapshot &snapshot : std::as_const(m_snapshots)) {
        const QJsonObject result = evaluate(PlasmaScripts::restore(snapshot.screen,snapshot.containmentId,
            snapshot.appliedPlugin,snapshot.sessionToken,snapshot.originalPlugin));
        if (!result.value(u"ok"_s).toBool()) remaining.push_back(snapshot);
    }
    m_snapshots = remaining;
    const bool active = !m_snapshots.isEmpty();
    if (active != m_active) { m_active = active; emit activeChanged(); }
    if (active) {
        saveRecovery();
        return false;
    }
    QFile::remove(m_sessionPath);
    m_ownerPid = 0;
    m_ownerIdentity.clear();
    setError({});
    return true;
}

bool PlasmaIntegration::updateActors(const QString &actorsJson, const QString &monitorName)
{
    const int screen = screenIndex(monitorName);
    const auto snapshot = std::find_if(m_snapshots.cbegin(),m_snapshots.cend(),[screen](const WallpaperSnapshot &s){ return s.screen==screen; });
    if (snapshot == m_snapshots.cend()) { setError(u"No composition is active on this monitor."_s); return false; }
    const auto document = QJsonDocument::fromJson(actorsJson.toUtf8());
    if (!document.isArray() || document.array().size()>64) { setError(u"Invalid animation list."_s); return false; }
    for (const auto &actor : document.array())
        if (!actor.isObject() || !localMedia(actor.toObject().value(u"url"_s).toString())) {
            setError(u"An animation file is unavailable."_s); return false;
        }
    const auto before = evaluate(PlasmaScripts::inspect(screen,snapshot->containmentId));
    if (!before.value(u"ok"_s).toBool()) return false;
    if (before.value(u"plugin"_s).toString()!=snapshot->appliedPlugin || before.value(u"sessionToken"_s).toString()!=snapshot->sessionToken) {
        setError(u"The wallpaper changed outside this app. Apply it again."_s); return false;
    }
    const QString json = QString::fromUtf8(document.toJson(QJsonDocument::Compact));
    const auto applied = evaluate(PlasmaScripts::apply(snapshot->containmentId,snapshot->appliedPlugin,
        snapshot->appliedPlugin,{{u"ActorsJson"_s,json}},snapshot->sessionToken));
    const auto after = evaluate(PlasmaScripts::inspect(screen,snapshot->containmentId));
    if (!applied.value(u"ok"_s).toBool() || after.value(u"actorsJson"_s).toString()!=json) return false;
    setError({});
    return true;
}

bool PlasmaIntegration::updateContentOptions(const QString &url, const QString &monitorName, const QString &json)
{
    if (m_mode!=Mode::Manager) return false;
    const auto document=QJsonDocument::fromJson(json.toUtf8());
    if (!document.isObject()) {setError(u"Invalid properties."_s);return false;}
    QJsonObject parameters;
    const auto options=document.object();
    const QStringList integerKeys{u"Speed"_s,u"Volume"_s};
    const QStringList boolKeys{u"AudioEnabled"_s,u"Playing"_s,u"Loop"_s};
    const QStringList stringKeys{u"FitMode"_s,u"VisualOptionsJson"_s};
    for (auto it=options.begin();it!=options.end();++it) {
        if (integerKeys.contains(it.key()) && it.value().isDouble())
            parameters.insert(it.key(),std::clamp(it.value().toInt(),it.key()==u"Speed"_s ? 10 : 0,it.key()==u"Speed"_s ? 300 : 100));
        else if (boolKeys.contains(it.key()) && it.value().isBool()) parameters.insert(it.key(),it.value());
        else if (stringKeys.contains(it.key()) && it.value().isString()) parameters.insert(it.key(),it.value());
        else {setError(u"Unsupported wallpaper property."_s);return false;}
    }
    const int screen=screenIndex(monitorName);
    const auto snapshot=std::find_if(m_snapshots.begin(),m_snapshots.end(),[screen](const auto &entry){return entry.screen==screen;});
    if (screen<0 || snapshot==m_snapshots.end()) {setError(u"No wallpaper is active on this monitor."_s);return false;}
    const auto before=evaluate(PlasmaScripts::inspect(screen,snapshot->containmentId));
    if (!before.value(u"ok"_s).toBool()) return false;
    if (before.value(u"contentUrl"_s).toString()!=url || before.value(u"plugin"_s).toString()!=snapshot->appliedPlugin ||
        before.value(u"sessionToken"_s).toString()!=snapshot->sessionToken) {setError(u"The active wallpaper changed outside this app."_s);return false;}
    const auto result=evaluate(PlasmaScripts::apply(snapshot->containmentId,snapshot->appliedPlugin,
        snapshot->appliedPlugin,parameters,snapshot->sessionToken));
    if (!result.value(u"ok"_s).toBool()) return false;
    setError({});return true;
}

bool PlasmaIntegration::updateAudioOptions(const QString &json)
{
    if (m_mode != Mode::Manager) return false;
    const auto document=QJsonDocument::fromJson(json.toUtf8());
    if (!document.isObject()) { setError(u"Invalid audio options."_s); return false; }
    const QString normalized=QString::fromUtf8(document.toJson(QJsonDocument::Compact));
    for (const auto &snapshot:m_snapshots) {
        const auto before=evaluate(PlasmaScripts::inspect(snapshot.screen,snapshot.containmentId));
        if (!before.value(u"ok"_s).toBool()) return false;
        if (before.value(u"plugin"_s).toString()!=snapshot.appliedPlugin ||
            before.value(u"sessionToken"_s).toString()!=snapshot.sessionToken) continue;
        const auto result=evaluate(PlasmaScripts::apply(snapshot.containmentId,snapshot.appliedPlugin,
            snapshot.appliedPlugin,{{u"AudioOptionsJson"_s,normalized}},snapshot.sessionToken));
        if (!result.value(u"ok"_s).toBool()) return false;
    }
    setError({});
    return true;
}

bool PlasmaIntegration::loadRecovery()
{
    QFile file(m_sessionPath);
    if (!file.exists()) return false;
    if (!file.open(QIODevice::ReadOnly)) { setError(u"Could not read desktop recovery data."_s); return false; }
    const QJsonObject root = QJsonDocument::fromJson(file.readAll()).object();
    if (root.isEmpty()) { setError(u"Invalid desktop recovery file."_s); return false; }
    m_ownerPid = root.value(u"ownerPid"_s).toInteger();
    m_ownerIdentity = root.value(u"ownerIdentity"_s).toString();
    for (const QJsonValue &value : root.value(u"screens"_s).toArray()) {
        const QJsonObject object = value.toObject();
        WallpaperSnapshot snapshot;
        snapshot.screen = object.value(u"screen"_s).toInt();
        snapshot.containmentId = object.value(u"containmentId"_s).toInt(-1);
        snapshot.originalPlugin = object.value(u"originalPlugin"_s).toString(object.value(u"plugin"_s).toString());
        snapshot.appliedPlugin = object.value(u"appliedPlugin"_s).toString(u"org.waniengine.wallpaper"_s);
        snapshot.sessionToken = object.value(u"sessionToken"_s).toString();
        if (!snapshot.originalPlugin.isEmpty()) m_snapshots.push_back(snapshot);
    }
    m_active = !m_snapshots.isEmpty();
    return m_active;
}

bool PlasmaIntegration::saveRecovery() const
{
    QJsonArray screens;
    for (const WallpaperSnapshot &snapshot : std::as_const(m_snapshots))
        screens.push_back(QJsonObject{{u"screen"_s,snapshot.screen},{u"containmentId"_s,snapshot.containmentId},
            {u"originalPlugin"_s,snapshot.originalPlugin},{u"appliedPlugin"_s,snapshot.appliedPlugin},{u"sessionToken"_s,snapshot.sessionToken}});
    const QJsonObject root{{u"schema"_s,2},{u"ownerPid"_s,m_ownerPid},{u"ownerIdentity"_s,m_ownerIdentity},{u"screens"_s,screens}};
    if (!QDir().mkpath(QFileInfo(m_sessionPath).absolutePath())) return false;
    QSaveFile file(m_sessionPath);
    if (!file.open(QIODevice::WriteOnly) || file.write(QJsonDocument(root).toJson(QJsonDocument::Indented))<0) return false;
    return file.commit();
}
void PlasmaIntegration::setError(const QString &message)
{
    if (message == m_lastError) return;
    m_lastError = message;
    emit lastErrorChanged();
}

int PlasmaIntegration::runGuardian(int argc, char **argv)
{
    QCoreApplication app(argc,argv);
    app.setApplicationName(u"waniengine"_s);
    app.setOrganizationName(u"WaniEngine"_s);
    if (argc != 5) return 2;
    const qint64 parentPid = QString::fromLocal8Bit(argv[2]).toLongLong();
    const QString identity = QString::fromLocal8Bit(argv[3]);
    if (parentPid <= 0 || identity.isEmpty()) return 2;
    PlasmaIntegration recovery(nullptr,Mode::Recovery);
    recovery.m_sessionPath = QString::fromLocal8Bit(argv[4]);
    recovery.m_snapshots.clear();
    if (!recovery.loadRecovery()) return 0;
    QElapsedTimer retryTime;
    QTimer timer;
    timer.setInterval(500);
    QObject::connect(&timer,&QTimer::timeout,&app,[&] {
        if (!QFileInfo::exists(recovery.m_sessionPath)) { app.exit(0); return; }
        if (processIdentity(parentPid)==identity) return;
        if (!retryTime.isValid()) retryTime.start();
        recovery.m_snapshots.clear();
        if (!recovery.loadRecovery()) { app.exit(0); return; }
        // A newly opened manager may already own this file. Never restore its session.
        if (recovery.m_ownerPid!=parentPid || recovery.m_ownerIdentity!=identity) { app.exit(0); return; }
        if (recovery.restore()) app.exit(0);
        else if (retryTime.elapsed()>60000) app.exit(1); // Keep the recovery file for the next launch.
    });
    timer.start();
    return app.exec();
}
