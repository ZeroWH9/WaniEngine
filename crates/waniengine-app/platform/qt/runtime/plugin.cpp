#include "runtime.h"
#include <QQmlExtensionPlugin>
#include <qqml.h>
class RuntimePlugin : public QQmlExtensionPlugin {
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QQmlExtensionInterface_iid)
public:
    void registerTypes(const char *uri) override {
        qmlRegisterType<FrameClock>(uri,1,0,"FrameClock");qmlRegisterType<VideoFrameLimiter>(uri,1,0,"VideoFrameLimiter");
        qmlRegisterType<CappedAnimation>(uri,1,0,"CappedAnimation");qmlRegisterType<DesktopPointerObserver>(uri,1,0,"DesktopPointerObserver");
    }
};
#include "plugin.moc"
