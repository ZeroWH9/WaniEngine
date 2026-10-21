#include "package_validator.h"
#ifdef WANIENGINE_HAS_KF6
#include <KConfigLoader>
#include <KConfigPropertyMap>
#endif
#include <QDBusMetaType>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QTemporaryDir>
#include <QUrl>
#include <QXmlStreamReader>
#include <memory>

using namespace Qt::StringLiterals;

QJsonObject validateWallpaperPackage(const QString &source)
{
    auto failure = [](const QString &message) { return QJsonObject{{u"ok"_s,false},{u"error"_s,message}}; };
    QTemporaryDir temporary;
    if (!temporary.isValid()) return failure(u"Could not create the validation directory."_s);
    QFile xml(source + u"/contents/config/main.xml"_s);
    if (!xml.open(QIODevice::ReadOnly)) return failure(u"Could not open the configuration schema."_s);
    const QByteArray schema = xml.readAll();
    QXmlStreamReader reader(schema);
    QStringList expectedKeys;
    while (!reader.atEnd()) {
        reader.readNext();
        if (reader.isStartElement() && reader.name() == u"entry") {
            const QString key = reader.attributes().value(u"name"_s).toString();
            if (key.isEmpty() || expectedKeys.contains(key)) return failure(u"The configuration schema contains an empty or duplicate key."_s);
            expectedKeys.push_back(key);
        }
    }
    if (reader.hasError() || expectedKeys.isEmpty()) return failure(u"Invalid wallpaper XML schema."_s);
    xml.close();

#ifdef WANIENGINE_HAS_KF6
    KConfigLoader loader(temporary.filePath(u"validationrc"_s),&xml);
    KConfigPropertyMap configuration(&loader);
    if (loader.items().size() != expectedKeys.size()) return failure(u"KDE did not recognize all configuration types."_s);
    auto validateMap = [&]() -> QString {
        for (const QString &key : configuration.keys()) {
            const QVariant value = configuration.value(key);
            if (!value.isValid() || !QDBusMetaType::typeToSignature(value.metaType()))
                return u"Configuration '%1' contains an invalid D-Bus value."_s.arg(key);
            if (!expectedKeys.contains(key) && !(key.endsWith(u"Default") && expectedKeys.contains(key.chopped(7))))
                return u"QML accessed a key that is missing from the schema: %1."_s.arg(key);
        }
        return {};
    };
    if (const QString error = validateMap(); !error.isEmpty()) return failure(error);
#endif

    QQmlEngine engine;
    QStringList warnings;
    QObject::connect(&engine,&QQmlEngine::warnings,&engine,[&](const QList<QQmlError> &errors) {
        for (const auto &error : errors) warnings.push_back(error.toString());
    });
    // Compile the Plasma wrapper without constructing a live WallpaperItem/containment.
    QQmlComponent wrapper(&engine,QUrl::fromLocalFile(source+u"/contents/ui/main.qml"_s));
    if (wrapper.isError()) return failure(wrapper.errorString());
    QQmlComponent renderer(&engine,QUrl::fromLocalFile(source+u"/contents/ui/WallpaperContent.qml"_s));
    if (renderer.isError()) return failure(renderer.errorString());

#ifdef WANIENGINE_HAS_KF6
    std::unique_ptr<QObject> object(renderer.createWithInitialProperties({{u"configuration"_s,QVariant::fromValue(&configuration)}}));
#else
    std::unique_ptr<QObject> object(renderer.createWithInitialProperties({{u"configuration"_s,QVariantMap{}}}));
#endif
    if (!object) return failure(renderer.errorString());
    if (!warnings.isEmpty()) return failure(warnings.join('\n'));
#ifdef WANIENGINE_HAS_KF6
    return {{u"ok"_s,true},{u"schemaKeys"_s,expectedKeys.size()},{u"qml"_s,u"compiled"_s},{u"config"_s,u"validated by KConfigLoader"_s}};
#else
    // Without KF6 we can still compile the QML and check the XML schema, but
    // the KConfig-backed value/type validation is unavailable. Report that
    // honestly instead of claiming the configuration was validated.
    return {{u"ok"_s,true},{u"schemaKeys"_s,expectedKeys.size()},{u"qml"_s,u"compiled"_s},{u"config"_s,u"schema and QML only (KConfig unavailable)"_s}};
#endif
}
