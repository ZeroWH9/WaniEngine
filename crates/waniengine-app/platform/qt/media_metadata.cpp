#include "media_metadata.h"
#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QProcess>
#include <QSaveFile>

using namespace Qt::StringLiterals;

QJsonObject describeLocalMedia(const QUrl &source, const QString &thumbnailCache)
{
    if (!source.isLocalFile() || !QFileInfo(source.toLocalFile()).isFile()) return {};
    const QString path = source.toLocalFile();
    QImageReader reader(path);
    if (reader.canRead()) return {{u"width"_s,reader.size().width()},{u"height"_s,reader.size().height()},
        {u"animated"_s,reader.supportsAnimation()},{u"hasAudio"_s,false}};
    QProcess probe;
    probe.start(u"ffprobe"_s,{u"-v"_s,u"error"_s,u"-show_entries"_s,
        u"stream=codec_type,width,height,avg_frame_rate:format=duration"_s,u"-of"_s,u"json"_s,path});
    if (!probe.waitForFinished(3000)) { probe.kill(); probe.waitForFinished(500); return {}; }
    if (probe.exitStatus()!=QProcess::NormalExit || probe.exitCode()!=0) return {};
    const auto info=QJsonDocument::fromJson(probe.readAllStandardOutput()).object();
    QJsonObject metadata;
    bool audio=false, video=false;
    for (const auto &value:info.value(u"streams"_s).toArray()) {
        const auto stream=value.toObject();
        if (stream.value(u"codec_type"_s)==u"video"_s) {
            video=true;
            const auto rate=stream.value(u"avg_frame_rate"_s).toString().split('/');
            if(rate.size()==2&&rate[1].toDouble()>0)metadata.insert(u"nativeFps"_s,rate[0].toDouble()/rate[1].toDouble());
            metadata.insert(u"width"_s,stream.value(u"width"_s));
            metadata.insert(u"height"_s,stream.value(u"height"_s));
        }
        if (stream.value(u"codec_type"_s)==u"audio"_s) audio=true;
    }
    if (!video) return {};
    metadata.insert(u"hasAudio"_s,audio);metadata.insert(u"animated"_s,true);
    metadata.insert(u"duration"_s,info.value(u"format"_s).toObject().value(u"duration"_s).toString().toDouble());

    // Prefer the local video project's existing preview. Otherwise cache the first decoded frame.
    const QString preview=QFileInfo(path).dir().filePath(u"preview.png"_s);
    QImageReader previewReader(preview);
    if (previewReader.canRead()) {
        metadata.insert(u"thumbnailUrl"_s,QUrl::fromLocalFile(preview).toString());
        return metadata;
    }
    const QFileInfo file(path);
    const QByteArray identity=(file.absoluteFilePath()+'\n'+QString::number(file.size())+'\n'+
        QString::number(file.lastModified().toMSecsSinceEpoch())).toUtf8();
    const QString thumbnail=QDir(thumbnailCache).filePath(QString::fromLatin1(
        QCryptographicHash::hash(identity,QCryptographicHash::Sha256).toHex())+u".png"_s);
    QImageReader cached(thumbnail);
    if (!cached.canRead() && QDir().mkpath(thumbnailCache)) {
        QProcess decoder;
        decoder.start(u"ffmpeg"_s,{u"-nostdin"_s,u"-v"_s,u"error"_s,u"-threads"_s,u"1"_s,
            u"-i"_s,path,u"-map"_s,u"0:v:0"_s,u"-vf"_s,
            u"scale=512:512:force_original_aspect_ratio=decrease"_s,u"-frames:v"_s,u"1"_s,
            u"-an"_s,u"-threads"_s,u"1"_s,u"-f"_s,u"image2pipe"_s,u"-vcodec"_s,u"png"_s,u"pipe:1"_s});
        if (decoder.waitForFinished(5000) && decoder.exitStatus()==QProcess::NormalExit && decoder.exitCode()==0) {
            const auto png=decoder.readAllStandardOutput();
            if (!png.isEmpty() && png.size()<4*1024*1024 && !QImage::fromData(png,"PNG").isNull()) {
                QSaveFile output(thumbnail);
                if (output.open(QIODevice::WriteOnly) && output.write(png)==png.size()) output.commit();
            }
        } else { decoder.kill(); decoder.waitForFinished(500); }
    }
    QImageReader ready(thumbnail);
    if (ready.canRead()) metadata.insert(u"thumbnailUrl"_s,QUrl::fromLocalFile(thumbnail).toString());
    return metadata;
}
