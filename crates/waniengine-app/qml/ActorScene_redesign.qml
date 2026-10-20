import QtQuick
import WaniEngine.Runtime 1.0 as Engine

Item {
    id: root
    property var actors: []
    // There are intentionally no input handlers: Qt sends an empty Wayland input region.
    Repeater {
        model: root.actors.length
        delegate: Item {
            id: actor
            required property int index
            readonly property var modelData: root.actors[index] || ({})
            readonly property real factor: Math.max(10,Math.min(300,Number(modelData.scale || 100))) / 100
            readonly property real nativeWidth: Math.max(1,media.sourceSize.width)
            readonly property real nativeHeight: Math.max(1,media.sourceSize.height)
            width: Math.min(root.width,nativeWidth * factor)
            height: width * nativeHeight / nativeWidth
            x: root.width * Math.max(0,Math.min(1,Number(modelData.x ?? 0.5))) - width / 2
            y: root.height * Math.max(0,Math.min(1,Number(modelData.y ?? 0.96))) - height
            opacity: Math.max(0,Math.min(1,Number(modelData.opacity ?? 100) / 100))
            rotation: Math.max(-180,Math.min(180,Number(modelData.rotation || 0)))
            transform: Scale { origin.x: actor.width/2; xScale: actor.modelData.flipX ? -1 : 1 }
            layer.enabled: Number(actor.modelData.samples || 0) > 0
            layer.samples: Number(actor.modelData.samples || 0)
            layer.textureSize: Qt.size(Math.max(1,Math.ceil(Math.min(width,root.width)*root.Screen.devicePixelRatio)),Math.max(1,Math.ceil(Math.min(height,root.height)*root.Screen.devicePixelRatio)))
            Engine.CappedAnimation {
                id: media
                objectName: "overlayAnimation"
                maximumSourceSize: Qt.size(Math.max(1,Math.ceil(Math.min(width,root.width)*root.Screen.devicePixelRatio)),Math.max(1,Math.ceil(Math.min(height,root.height)*root.Screen.devicePixelRatio)))
                anchors.fill: parent
                source: actor.modelData.url || ""
                fillMode: Image.PreserveAspectFit
                smooth: actor.modelData.smoothing !== false
                speed: Math.max(0.1,Math.min(3,Number(actor.modelData.speed || 100)/100))
                paused: actor.modelData.playing === false
                repeat: actor.modelData.loop !== false
                fps: Number(actor.modelData.fpsLimit || 60)
            }
        }
    }
}
