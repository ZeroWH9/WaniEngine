#pragma once

#include <QObject>
#include <QVariantMap>

class MediaOptimizer final : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;

    // Build a cached, display-sized video copy when the original exceeds the
    // selected monitor. The source in the user's library is never modified.
    Q_INVOKABLE QVariantMap prepareVideo(const QString &sourceUrl, int sourceWidth, int sourceHeight,
                                         int targetWidth, int targetHeight) const;
};
