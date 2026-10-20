#include "plasma_scripts.h"
#include <QJsonDocument>

namespace {
QString wrap(const QJsonObject &parameters, const QString &body)
{
    // JSON escaping keeps paths and names out of executable script syntax.
    return QStringLiteral("(function(){var p=%1;try{%2}catch(e){print('WANIENGINE_JSON:'+JSON.stringify({ok:false,error:String(e)}));}})();")
        .arg(QString::fromUtf8(QJsonDocument(parameters).toJson(QJsonDocument::Compact)), body);
}
}

QString PlasmaScripts::resolveScreen(const QRect &geometry)
{
    return wrap({{QStringLiteral("x"),geometry.x()},{QStringLiteral("y"),geometry.y()},
                 {QStringLiteral("width"),geometry.width()},{QStringLiteral("height"),geometry.height()}},QStringLiteral(R"JS(
        var ids = [], candidates = [];
        var all = desktops();
        for (var i=0;i<all.length;i++) {
            var id = Number(all[i].screen);
            if (id>=0 && ids.indexOf(id)<0) ids.push(id);
        }
        for (var i=0;i<ids.length;i++) {
            var g = screenGeometry(ids[i]);
            if (g.x===p.x && g.y===p.y && g.width===p.width && g.height===p.height) candidates.push(ids[i]);
        }
        if (candidates.length!==1) throw new Error('Plasma could not uniquely identify this monitor.');
        print('WANIENGINE_JSON:'+JSON.stringify({ok:true,screen:candidates[0]}));
    )JS"));
}

QString PlasmaScripts::inspect(int screen, int containmentId)
{
    return wrap({{QStringLiteral("screen"), screen}, {QStringLiteral("id"), containmentId}}, QStringLiteral(R"JS(
        var d = p.id >= 0 ? desktopById(p.id) : desktopForScreen(p.screen);
        if (!d) throw new Error('No desktop exists for the selected monitor.');
        var plugin = String(d.wallpaperPlugin);
        d.currentConfigGroup = ['Wallpaper', plugin, 'General'];
        function text(key, fallback) {
            var value = d.readConfig(key, fallback);
            return value === undefined || value === null ? String(fallback) : String(value);
        }
        var result = {ok:true,id:Number(d.id),screen:Number(d.screen),plugin:plugin,
            image:text('Image',''),color:text('Color','#202020'),
            contentUrl:text('ContentUrl','none'),contentKind:text('ContentKind','color'),
            actorsJson:text('ActorsJson','[]'),sessionToken:text('SessionToken','inactive'),
            fitMode:text('FitMode','cover'),speed:Number(d.readConfig('Speed',100)),
            volume:Number(d.readConfig('Volume',50)),audioEnabled:!!d.readConfig('AudioEnabled',false),
            audioOptionsJson:text('AudioOptionsJson','{}')};
        print('WANIENGINE_JSON:'+JSON.stringify(result));
    )JS"));
}

QString PlasmaScripts::apply(int containmentId, const QString &expectedPlugin,
                            const QString &plugin, const QJsonObject &parameters, const QString &expectedToken)
{
    return wrap({{QStringLiteral("id"), containmentId}, {QStringLiteral("expected"), expectedPlugin},
                 {QStringLiteral("plugin"), plugin}, {QStringLiteral("parameters"), parameters},
                 {QStringLiteral("token"), expectedToken}}, QStringLiteral(R"JS(
        var d = desktopById(p.id);
        if (!d) throw new Error('The desktop was not found.');
        if (String(d.wallpaperPlugin) !== p.expected)
            throw new Error('The wallpaper changed while applying. Try again.');
        d.currentConfigGroup = ['Wallpaper', p.expected, 'General'];
        if (p.token && String(d.readConfig('SessionToken','inactive')) !== p.token)
            throw new Error('The composition changed outside the app.');
        d.currentConfigGroup = ['Wallpaper', p.plugin, 'General'];
        for (var key in p.parameters) {
            if (Object.prototype.hasOwnProperty.call(p.parameters,key)) d.writeConfig(key,p.parameters[key]);
        }
        d.wallpaperPlugin = p.plugin;
        d.reloadConfig();
        print('WANIENGINE_JSON:'+JSON.stringify({ok:true,plugin:String(d.wallpaperPlugin),id:Number(d.id)}));
    )JS"));
}

QString PlasmaScripts::restore(int screen, int containmentId, const QString &appliedPlugin,
                              const QString &sessionToken, const QString &originalPlugin)
{
    return wrap({{QStringLiteral("screen"), screen}, {QStringLiteral("id"), containmentId},
                 {QStringLiteral("applied"), appliedPlugin}, {QStringLiteral("token"), sessionToken},
                 {QStringLiteral("original"), originalPlugin}}, QStringLiteral(R"JS(
        var d = p.id >= 0 ? desktopById(p.id) : desktopForScreen(p.screen);
        if (!d) throw new Error('The desktop is not yet available for restoration.');
        if (String(d.wallpaperPlugin) !== p.applied) {
            print('WANIENGINE_JSON:'+JSON.stringify({ok:true,skipped:true,plugin:String(d.wallpaperPlugin)}));
            return;
        }
        d.currentConfigGroup = ['Wallpaper', p.applied, 'General'];
        if (p.token && String(d.readConfig('SessionToken','inactive')) !== p.token) {
            print('WANIENGINE_JSON:'+JSON.stringify({ok:true,skipped:true,plugin:String(d.wallpaperPlugin)}));
            return;
        }
        // The original plugin's group was never edited.
        d.wallpaperPlugin = p.original;
        d.reloadConfig();
        if (String(d.wallpaperPlugin) !== p.original) throw new Error('Plasma did not confirm restoration.');
        print('WANIENGINE_JSON:'+JSON.stringify({ok:true,restored:true,plugin:String(d.wallpaperPlugin)}));
    )JS"));
}

QJsonObject PlasmaScripts::parseResult(const QString &output)
{
    const QString marker = QStringLiteral("WANIENGINE_JSON:");
    const qsizetype position = output.lastIndexOf(marker);
    if (position < 0)
        return {{QStringLiteral("ok"), false}, {QStringLiteral("error"), QStringLiteral("Plasma did not confirm the operation.")}};
    QJsonParseError error;
    const QJsonDocument document = QJsonDocument::fromJson(output.mid(position + marker.size()).trimmed().toUtf8(), &error);
    if (error.error != QJsonParseError::NoError || !document.isObject())
        return {{QStringLiteral("ok"), false}, {QStringLiteral("error"), QStringLiteral("Plasma returned an invalid confirmation.")}};
    return document.object();
}
