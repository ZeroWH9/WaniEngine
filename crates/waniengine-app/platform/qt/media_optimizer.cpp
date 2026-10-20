#include "media_optimizer.h"

#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QProcess>
#include <QLockFile>
#include <QSize>
#include <QStandardPaths>
#include <QStringList>
#include <QUrl>
#include <QUuid>

namespace {
QVariantMap failure(const QString &message)
{
    return {{QStringLiteral("ok"), false}, {QStringLiteral("error"), message}};
}

QString fingerprint(QFile &source, const QSize &outputSize)
{
    QCryptographicHash hash(QCryptographicHash::Sha256);
    if (!source.open(QIODevice::ReadOnly)) return {};
    QByteArray buffer(1024 * 1024, Qt::Uninitialized);
    while (!source.atEnd()) {
        const qint64 count = source.read(buffer.data(), buffer.size());
        if (count < 0) return {};
        if (count == 0) break;
        hash.addData(QByteArrayView(buffer.constData(), count));
    }
    hash.addData(QByteArray::number(outputSize.width()));
    hash.addData("x");
    hash.addData(QByteArray::number(outputSize.height()));
    return QString::fromLatin1(hash.result().toHex());
}
}

QVariantMap MediaOptimizer::prepareVideo(const QString &sourceUrl, int sourceWidth, int sourceHeight,
                                         int targetWidth, int targetHeight) const
{
    const QUrl parsed(sourceUrl);
    if (!parsed.isLocalFile()) return failure(QStringLiteral("The video must be a local file."));
    const QString sourcePath = parsed.toLocalFile();
    const QFileInfo info(sourcePath);
    if (!info.isFile() || !info.isReadable()) return failure(QStringLiteral("The video file is unavailable."));
    if (targetWidth <= 0 || targetHeight <= 0)
        return failure(QStringLiteral("Could not detect the video or monitor resolution."));
    if (sourceWidth <= 0 || sourceHeight <= 0) {
        const QString ffprobe = QStandardPaths::findExecutable(QStringLiteral("ffprobe"));
        if (ffprobe.isEmpty()) return failure(QStringLiteral("Could not detect video resolution: ffprobe is unavailable."));
        QProcess probe;
        probe.start(ffprobe,{QStringLiteral("-v"),QStringLiteral("error"),QStringLiteral("-select_streams"),QStringLiteral("v:0"),
            QStringLiteral("-show_entries"),QStringLiteral("stream=width,height"),QStringLiteral("-of"),QStringLiteral("csv=s=x:p=0"),sourcePath});
        if (!probe.waitForStarted(3000) || !probe.waitForFinished(10000) || probe.exitCode()!=0)
            return failure(QStringLiteral("ffprobe could not detect the video resolution."));
        const QStringList dimensions=QString::fromUtf8(probe.readAllStandardOutput()).trimmed().split(u'x');
        if (dimensions.size()!=2) return failure(QStringLiteral("ffprobe returned an invalid resolution."));
        sourceWidth=dimensions[0].toInt();sourceHeight=dimensions[1].toInt();
    }
    if (sourceWidth <= 0 || sourceHeight <= 0)
        return failure(QStringLiteral("Could not detect the video resolution."));
    if (sourceWidth <= targetWidth && sourceHeight <= targetHeight)
        return {{QStringLiteral("ok"), true}, {QStringLiteral("url"), sourceUrl},
                {QStringLiteral("optimized"), false}, {QStringLiteral("width"), sourceWidth},
                {QStringLiteral("height"), sourceHeight}};

    const double scale = qMin(1.0, qMin(double(targetWidth) / sourceWidth, double(targetHeight) / sourceHeight));
    QSize outputSize(qMax(2, int(sourceWidth * scale) & ~1), qMax(2, int(sourceHeight * scale) & ~1));
    QFile source(sourcePath);
    const QString key = fingerprint(source, outputSize);
    if (key.isEmpty()) return failure(QStringLiteral("Could not identify the video."));

    const QString cacheRoot = QStandardPaths::writableLocation(QStandardPaths::CacheLocation)
        + QStringLiteral("/optimized-wallpapers");
    if (!QDir().mkpath(cacheRoot)) return failure(QStringLiteral("Could not create the optimized video cache."));
    const QString destination = QDir(cacheRoot).filePath(key + QStringLiteral(".mp4"));
    QLockFile cacheLock(destination + QStringLiteral(".lock"));
    cacheLock.setStaleLockTime(0);
    if (!cacheLock.tryLock(180000)) return failure(QStringLiteral("Another video preparation is still using this cache entry."));
    if (QFileInfo(destination).isFile() && QFileInfo(destination).size() > 0)
        return {{QStringLiteral("ok"), true}, {QStringLiteral("url"), QUrl::fromLocalFile(destination).toString()},
                {QStringLiteral("optimized"), true}, {QStringLiteral("cacheHit"), true},
                {QStringLiteral("width"), outputSize.width()}, {QStringLiteral("height"), outputSize.height()}};

    const QString ffmpeg = QStandardPaths::findExecutable(QStringLiteral("ffmpeg"));
    if (ffmpeg.isEmpty()) return failure(QStringLiteral("FFmpeg is unavailable; the oversized video was not applied."));
    const QString temporary = QDir(cacheRoot).filePath(key + QStringLiteral("-")
        + QUuid::createUuid().toString(QUuid::WithoutBraces) + QStringLiteral(".partial.mp4"));
    QProcess encoder;
    encoder.setProgram(ffmpeg);
    const QStringList arguments{QStringLiteral("-hide_banner"), QStringLiteral("-loglevel"), QStringLiteral("error"),
        QStringLiteral("-nostdin"), QStringLiteral("-y"), QStringLiteral("-i"), sourcePath,
        QStringLiteral("-map"), QStringLiteral("0:v:0"), QStringLiteral("-map"), QStringLiteral("0:a?"),
        QStringLiteral("-sn"), QStringLiteral("-dn"), QStringLiteral("-vf"),
        QStringLiteral("scale=%1:%2:flags=lanczos").arg(outputSize.width()).arg(outputSize.height()),
        QStringLiteral("-c:v"), QStringLiteral("libx264"), QStringLiteral("-preset"), QStringLiteral("veryfast"),
        QStringLiteral("-crf"), QStringLiteral("20"), QStringLiteral("-pix_fmt"), QStringLiteral("yuv420p"),
        QStringLiteral("-c:a"), QStringLiteral("aac"), QStringLiteral("-b:a"), QStringLiteral("192k"),
        QStringLiteral("-movflags"), QStringLiteral("+faststart"), temporary};
    encoder.setArguments(arguments);
    encoder.start();
    if (!encoder.waitForStarted(5000)) return failure(QStringLiteral("Could not start FFmpeg."));
    if (!encoder.waitForFinished(180000)) {
        encoder.kill();
        encoder.waitForFinished(3000);
        QFile::remove(temporary);
        return failure(QStringLiteral("Video optimization exceeded three minutes and was cancelled."));
    }
    const QString diagnostic = QString::fromUtf8(encoder.readAllStandardError()).trimmed();
    if (encoder.exitStatus() != QProcess::NormalExit || encoder.exitCode() != 0
        || !QFileInfo(temporary).isFile() || QFileInfo(temporary).size() == 0) {
        QFile::remove(temporary);
        return failure(QStringLiteral("FFmpeg could not generate the display-sized video: %1")
            .arg(diagnostic.left(500)));
    }
    if (QFileInfo::exists(destination)) QFile::remove(destination);
    if (!QFile::rename(temporary, destination)) {
        QFile::remove(temporary);
        return failure(QStringLiteral("Could not save the optimized video."));
    }
    return {{QStringLiteral("ok"), true}, {QStringLiteral("url"), QUrl::fromLocalFile(destination).toString()},
            {QStringLiteral("optimized"), true}, {QStringLiteral("cacheHit"), false},
            {QStringLiteral("width"), outputSize.width()}, {QStringLiteral("height"), outputSize.height()}};
}
