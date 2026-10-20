#pragma once
#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QString>
inline QString waniEngineRuntimeRoot() {
    const QString installed=QDir(QCoreApplication::applicationDirPath()).absoluteFilePath("../lib/qml");
    if(QFileInfo::exists(installed+"/WaniEngine/Runtime/qmldir"))return installed;
    return QString::fromUtf8(WANIENGINE_RUNTIME_ROOT);
}
inline QString waniEngineWallpaperSource() {
    const QString installed=QDir(QCoreApplication::applicationDirPath()).absoluteFilePath("../share/waniengine/wallpaper");
    if(QFileInfo::exists(installed+"/metadata.json"))return installed;
    return QString::fromUtf8(WANIENGINE_PLASMA_PACKAGE_SOURCE);
}
