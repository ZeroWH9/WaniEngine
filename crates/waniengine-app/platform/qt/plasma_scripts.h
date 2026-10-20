#pragma once
#include <QJsonObject>
#include <QString>
#include <QRect>
namespace PlasmaScripts {
QString resolveScreen(const QRect &geometry);
QString inspect(int screen, int containmentId = -1);
QString apply(int containmentId, const QString &expectedPlugin, const QString &plugin, const QJsonObject &parameters, const QString &expectedToken = {});
QString restore(int screen, int containmentId, const QString &appliedPlugin, const QString &sessionToken, const QString &originalPlugin);
QJsonObject parseResult(const QString &output);
}
