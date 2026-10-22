import QtQuick
import QtMultimedia
import "runtime" as Engine

Item {
    id: root
    required property var configuration
    clip: true
    readonly property string mediaSource: configuration.ContentUrl === "none" ? "" : String(configuration.ContentUrl || "")
    readonly property string contentKind: String(configuration.ContentKind || "color")
    readonly property bool isVideo: contentKind === "video"
    readonly property bool isAnimated: contentKind === "animated"
    readonly property real speed: Math.max(0.1, Math.min(3, Number(configuration.Speed || 100) / 100))
    readonly property bool running: configuration.Playing !== false
    readonly property bool repeat: configuration.Loop !== false
    readonly property var visual: parseVisual(configuration.VisualOptionsJson)
    readonly property var audioOptions: parseVisual(configuration.AudioOptionsJson)
    // The containing Plasma DesktopView receives focus when the user clicks the desktop.
    // Observe its state; never request focus or intercept desktop input.
    property bool desktopActive: Window.active
    readonly property bool audioAllowed: desktopActive || audioOptions.managerActive === true
    readonly property int fpsLimit: Math.max(15, Math.min(Math.round(Screen.refreshRate || 60), Number(visual.fpsLimit || 60)))
    Engine.DesktopPointerObserver { window: root.Window.window }
    MediaDevices { id: mediaDevices }
    readonly property var audioDevice: {
        const id = String(audioOptions.deviceId || "")
        for (let i = 0; i < mediaDevices.audioOutputs.length; ++i) {
            if (String(mediaDevices.audioOutputs[i].id) === id) return mediaDevices.audioOutputs[i]
        }
        return mediaDevices.defaultAudioOutput
    }
    readonly property real zoom: Math.max(1,Math.min(2,Number(visual.zoom || 100)/100))
    readonly property real motionAmount: Math.max(0,Math.min(0.1,Number(visual.motionAmount ?? 2)/100))
    property real motionProgress: 0
    function parseVisual(json) {
        try { const value=JSON.parse(json || "{}"); return value && !Array.isArray(value) && typeof value === "object" ? value : {} }
        catch(error) { return {} }
    }
    Engine.FrameClock {
        id: motionClock
        objectName: "wallpaperFrameClock"
        fps: root.fpsLimit
        running: root.running && !root.isVideo && !root.isAnimated && root.visual.motion !== "none" && root.visual.motion !== undefined
        onTick: root.motionProgress = (1 - Math.cos(Math.PI * seconds / Math.max(2,Math.min(60,Number(root.visual.motionPeriod || 12))))) / 2
    }

    function imageFillMode() {
        switch (configuration.FitMode) {
        case "contain": return Image.PreserveAspectFit
        case "stretch": return Image.Stretch
        case "center": return Image.Pad
        default: return Image.PreserveAspectCrop
        }
    }
    Rectangle { anchors.fill: parent; color: root.configuration.BackgroundColor || "#202020" }
    Item {
        id: visualLayer
        anchors.fill: parent
        layer.enabled: Number(root.visual.samples || 0) > 0
        layer.samples: Number(root.visual.samples || 0)
        readonly property real motionZoom: root.visual.motion === "zoom" ? root.motionProgress*root.motionAmount : root.visual.motion === "pan" ? root.motionAmount : 0
        x: 0
        transform: [
            Scale { origin.x: root.width/2; origin.y: root.height/2; xScale: (root.visual.flipX ? -1 : 1)*(root.zoom+visualLayer.motionZoom); yScale: root.zoom+visualLayer.motionZoom },
            Translate { x: Math.max(-100,Math.min(100,Number(root.visual.offsetX || 0)))*root.width/200 + (root.visual.motion === "pan" ? (root.motionProgress-0.5)*root.width*root.motionAmount : 0); y: Math.max(-100,Math.min(100,Number(root.visual.offsetY || 0)))*root.height/200 }
        ]
    Image {
        objectName: "wallpaperImage"
        anchors.fill: parent
        visible: !root.isVideo && !root.isAnimated
        source: visible ? root.mediaSource : ""
        fillMode: root.imageFillMode()
        asynchronous: true
        smooth: root.visual.smoothing !== false
        mipmap: false
        sourceSize: Qt.size(Math.max(1,Math.ceil(root.width*root.Screen.devicePixelRatio)),Math.max(1,Math.ceil(root.height*root.Screen.devicePixelRatio)))
    }
    Engine.CappedAnimation {
        objectName: "wallpaperAnimation"
        maximumSourceSize: Qt.size(Math.max(1,Math.ceil(root.width*root.Screen.devicePixelRatio)),Math.max(1,Math.ceil(root.height*root.Screen.devicePixelRatio)))
        anchors.fill: parent
        visible: root.isAnimated
        source: visible ? root.mediaSource : ""
        fillMode: root.imageFillMode()
        speed: root.speed
        smooth: root.visual.smoothing !== false
        paused: !root.running
        repeat: root.repeat
        fps: root.fpsLimit
    }
    Loader {
        anchors.fill: parent
        active: root.isVideo
        sourceComponent: Component {
            VideoOutput {
                id: videoOutput
                objectName: "wallpaperVideo"
                anchors.centerIn: parent
                width: root.configuration.FitMode === "center" && sourceRect.width > 0 ? sourceRect.width : root.width
                height: root.configuration.FitMode === "center" && sourceRect.height > 0 ? sourceRect.height : root.height
                visible: root.isVideo
                fillMode: root.configuration.FitMode === "stretch" ? VideoOutput.Stretch
                          : root.configuration.FitMode === "contain" || root.configuration.FitMode === "center"
                            ? VideoOutput.PreserveAspectFit : VideoOutput.PreserveAspectCrop

            Connections {
                target: root
                function onRunningChanged() {
                    if (root.running) player.play()
                    else { player.stop(); frameLimiter.releaseFrame() }
                }
            }
            Engine.VideoFrameLimiter { id: frameLimiter; objectName: "wallpaperFrameLimiter"; fps: root.fpsLimit; target: videoOutput.videoSink }
            MediaPlayer {
                id: player
                objectName: "wallpaperPlayer"
                source: root.isVideo ? root.mediaSource : ""
                videoOutput: frameLimiter
                loops: root.repeat ? MediaPlayer.Infinite : 1
                playbackRate: root.speed
                audioOutput: AudioOutput {
                    objectName: "wallpaperAudio"
                    device: root.audioDevice
                    muted: !root.configuration.AudioEnabled || !root.audioAllowed
                    volume: Math.max(0,Math.min(1,Number(root.configuration.Volume ?? 50) / 100))
                          * Math.max(0,Math.min(1,Number(root.audioOptions.globalVolume ?? 100) / 100))
                }
                onSourceChanged: {
                    frameLimiter.releaseFrame()
                    if (root.isVideo && source.toString().length > 0)
                        Qt.callLater(function() { if(root.running) player.play() })
                }
                Component.onCompleted: {
                    if (root.isVideo && root.running && source.toString().length > 0) play()
                    else frameLimiter.releaseFrame()
                }
            }
            }
        }
    }
    }
}
