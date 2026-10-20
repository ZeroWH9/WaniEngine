import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import WaniEngine.Runtime

ColumnLayout {
    id: root
    property var assets: []
    property var selectedAsset: null
    property var activeUrls: []
    property var monitorNames: []
    property var monitorLabels: []
    property string selectedMonitor: ""
    property bool animate: true
    property var backgroundAsset: null
    property int fpsLimit: 60

    signal selected(string url)
    signal monitorSelected(string name)
    signal importRequested()
    signal addAudioRequested()
    signal soundSettingsRequested(int index)
    signal updateRequested(string url, string key, var value)
    signal audioEnabledRequested(bool enabled)
    signal applyRequested()
    signal removeRequested()
    signal deleteRequested()

    spacing: 16

    RowLayout {
        Layout.fillWidth: true
        Layout.preferredHeight: 56
        spacing: 12

        ColumnLayout {
            Layout.preferredWidth: 330
            spacing: 0
            Text {
                text: "Animations"
                color: "#F4F2FA"
                font.pixelSize: 26
                font.weight: Font.DemiBold
            }
            Text {
                text: "Characters on your desktop"
                color: "#9993AB"
                font.pixelSize: 12
            }
        }

        Item { Layout.fillWidth: true }

        TextField {
            id: characterSearch
            objectName: "characterSearch"
            Layout.preferredWidth: 320
            Layout.preferredHeight: 40
            placeholderText: "Search characters…"
            text: root.searchText
            color: "#F4F2FA"
            placeholderTextColor: "#9993AB"
            font.pixelSize: 14
            selectByMouse: true
            onTextEdited: root.searchEdited(text)
            background: Rectangle {
                color: "#16141F"
                radius: 10
                border.color: characterSearch.activeFocus ? "#7872C9" : "#383341"
            }
        }

        Button_redesign {
            text: "Add animation"
            Layout.preferredWidth: 224
            Layout.preferredHeight: 48
            onClicked: root.importRequested()
            background: Rectangle { radius: 10; color: parent.down ? "#6862B7" : parent.hovered ? "#A19BDC" : "#7872C9" }
            contentItem: Text {
                text: parent.text
                color: "#F4F2FA"
                font.pixelSize: 14
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }
    }

    signal searchEdited(string text)

    Item {
        id: desktopPreview
        objectName: "characterDesktopPreview"
        Layout.fillWidth: true
        Layout.preferredHeight: 232
        Layout.minimumHeight: 192
        clip: true

        readonly property var selectedBackground: root.backgroundAsset
        readonly property real fitScale: Math.min(width / Math.max(1, root.screenWidth), height / Math.max(1, root.screenHeight))
        readonly property real desktopWidth: root.screenWidth * fitScale
        readonly property real desktopHeight: root.screenHeight * fitScale
        readonly property real desktopX: (width - desktopWidth) / 2
        readonly property real desktopY: (height - desktopHeight) / 2
        readonly property real actorFactor: Math.max(10, Math.min(300, Number(root.selectedAsset ? root.selectedAsset.scale || 100 : 100))) / 100
        readonly property real actorNativeWidth: Math.max(1, Number(root.selectedAsset ? root.selectedAsset.width || 240 : 240))
        readonly property real actorNativeHeight: Math.max(1, Number(root.selectedAsset ? root.selectedAsset.height || 320 : 320))
        readonly property real actorWidth: Math.min(root.screenWidth, actorNativeWidth * actorFactor) * fitScale
        readonly property real actorHeight: actorWidth * actorNativeHeight / actorNativeWidth
        readonly property real actorCenterX: desktopX + desktopWidth * Number(root.selectedAsset ? root.selectedAsset.posX ?? 50 : 50) / 100
        readonly property real actorBottomY: desktopY + desktopHeight * Number(root.selectedAsset ? root.selectedAsset.posY ?? 96 : 96) / 100

        Rectangle {
            anchors.fill: parent
            radius: 10
            color: "#16141F"
            border.color: "#383341"
        }

        Rectangle {
            x: desktopPreview.desktopX
            y: desktopPreview.desktopY
            width: desktopPreview.desktopWidth
            height: desktopPreview.desktopHeight
            color: "#1A1822"
            clip: true

            Image {
                sourceSize: Qt.size(Math.max(1,Math.ceil(width*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(height*(root.Screen.devicePixelRatio || 1))))
                anchors.fill: parent
                source: desktopPreview.selectedBackground ? desktopPreview.selectedBackground.thumbnailUrl || desktopPreview.selectedBackground.url : ""
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
            }
            Rectangle {
                anchors.fill: parent
                color: "#33000000"
                visible: desktopPreview.selectedBackground !== null
            }
            Text {
                anchors.centerIn: parent
                visible: desktopPreview.selectedBackground === null
                text: "Desktop preview"
                color: "#9993AB"
                font.pixelSize: 14
            }
        }

        CappedAnimation {
            id: actorPreview
            objectName: "characterPreview"
            maximumSourceSize: Qt.size(Math.max(1,Math.ceil(Math.min(width,desktopPreview.width)*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(Math.min(height,desktopPreview.height)*(root.Screen.devicePixelRatio || 1))))
            visible: root.selectedAsset !== null
            x: desktopPreview.actorCenterX - width / 2
            y: desktopPreview.actorBottomY - height
            width: desktopPreview.actorWidth
            height: desktopPreview.actorHeight
            source: root.selectedAsset ? root.selectedAsset.url : ""
            fillMode: Image.PreserveAspectFit
            paused: !root.animate || !root.selectedAsset || root.selectedAsset.playing === false
            repeat: root.selectedAsset ? root.selectedAsset.loop !== false : true
            fps: root.fpsLimit
            smooth: root.selectedAsset ? root.selectedAsset.smoothing !== false : true
            speed: root.selectedAsset ? Number(root.selectedAsset.speed || 100) / 100 : 1
            opacity: root.selectedAsset ? Number(root.selectedAsset.opacity ?? 100) / 100 : 1
            rotation: root.selectedAsset ? Number(root.selectedAsset.rotation || 0) : 0
            transform: Scale { origin.x: actorPreview.width / 2; xScale: root.selectedAsset && root.selectedAsset.flipX ? -1 : 1 }
        }

        Rectangle {
            visible: root.selectedAsset !== null
            x: desktopPreview.actorCenterX - actorPreview.width / 2 - 2
            y: desktopPreview.actorBottomY - actorPreview.height - 2
            width: actorPreview.width + 4
            height: actorPreview.height + 4
            color: "transparent"
            border.color: "#7872C9"
            border.width: 1
            opacity: 0.8
        }

        Rectangle {
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.margins: 12
            width: previewLabel.implicitWidth + 28
            height: 36
            radius: 18
            color: "#D916141F"
            Text {
                id: previewLabel
                anchors.centerIn: parent
                text: "Desktop preview"
                color: "#F4F2FA"
                font.pixelSize: 12
            }
        }

        ComboBox_redesign {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 12
            width: Math.min(256, Math.max(176, parent.width * 0.2))
            height: 40
            model: root.monitorLabels
            currentIndex: Math.max(0, root.monitorNames.indexOf(root.selectedMonitor))
            onActivated: root.monitorSelected(root.monitorNames[index])
            contentItem: Text { leftPadding: 12; rightPadding: 28; text: parent.displayText; color: "#F4F2FA"; font.pixelSize: 13; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
            background: Rectangle { radius: 10; color: "#D916141F"; border.color: "#383341" }
        }

        Button_redesign {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 12
            width: 44
            height: 44
            text: root.animate ? "Ⅱ" : "▶"
            onClicked: root.animate = !root.animate
            background: Rectangle { radius: 22; color: "#D916141F"; border.color: "#383341" }
            contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 16; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
        }
    }

    Rectangle {
        Layout.fillWidth: true
        Layout.preferredHeight: 60
        radius: 10
        color: "#1A1822"
        border.color: "#383341"
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            spacing: 12
            Image {
                sourceSize: Qt.size(Math.max(1,Math.ceil(width*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(height*(root.Screen.devicePixelRatio || 1))))
                Layout.preferredWidth: 40
                Layout.preferredHeight: 40
                source: root.selectedAsset ? root.selectedAsset.thumbnailUrl || root.selectedAsset.url : ""
                fillMode: Image.PreserveAspectFit
                asynchronous: true
            }
            ColumnLayout {
                spacing: 0
                Text { text: root.selectedAsset ? root.selectedAsset.name : "No character selected"; color: "#F4F2FA"; font.pixelSize: 15; font.weight: Font.DemiBold; elide: Text.ElideRight }
                Text { text: root.selectedAsset && root.activeUrls.indexOf(root.selectedAsset.url) >= 0 ? "Active · Changes apply in real time" : "Inactive · Select a character below"; color: root.selectedAsset && root.activeUrls.indexOf(root.selectedAsset.url) >= 0 ? "#34D399" : "#9993AB"; font.pixelSize: 12 }
            }
            Item { Layout.fillWidth: true }
            Button_redesign {
                enabled: root.selectedAsset !== null
                text: root.selectedAsset && root.activeUrls.indexOf(root.selectedAsset.url) >= 0 ? "Disable" : "Enable"
                Layout.preferredWidth: 164
                Layout.preferredHeight: 40
                onClicked: root.applyRequested()
                background: Rectangle { radius: 10; color: "transparent"; border.color: parent.enabled && root.selectedAsset && root.activeUrls.indexOf(root.selectedAsset.url) >= 0 ? "#FB7185" : "#383341" }
                contentItem: Text { text: parent.text; color: root.selectedAsset && root.activeUrls.indexOf(root.selectedAsset.url) >= 0 ? "#FB7185" : "#7872C9"; font.pixelSize: 13; font.weight: Font.DemiBold; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.preferredHeight: 248
        spacing: 16

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumWidth: 0
            radius: 10
            color: "#1A1822"
            border.color: "#383341"
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 4
                Text { text: "Transform"; color: "#F4F2FA"; font.pixelSize: 14; font.weight: Font.DemiBold }
                PropertySlider_redesign { Layout.fillWidth: true; Layout.preferredHeight: 36; label: "Size"; from: 25; to: 200; stepSize: 5; defaultValue: 100; value: root.selectedAsset ? root.selectedAsset.scale ?? 100 : 100; suffix: "%"; enabled: root.selectedAsset !== null; onEdited: value => root.updateSelected("scale", value) }
                PropertySlider_redesign { Layout.fillWidth: true; Layout.preferredHeight: 36; label: "Position X"; from: 0; to: 100; defaultValue: 50; value: root.selectedAsset ? root.selectedAsset.posX ?? 50 : 50; suffix: "%"; enabled: root.selectedAsset !== null; onEdited: value => root.updateSelected("posX", value) }
                PropertySlider_redesign { Layout.fillWidth: true; Layout.preferredHeight: 36; label: "Position Y"; from: 0; to: 100; defaultValue: 96; value: root.selectedAsset ? root.selectedAsset.posY ?? 96 : 96; suffix: "%"; enabled: root.selectedAsset !== null; onEdited: value => root.updateSelected("posY", value) }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumWidth: 0
            radius: 10
            color: "#1A1822"
            border.color: "#383341"
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 4
                Text { text: "Appearance"; color: "#F4F2FA"; font.pixelSize: 14; font.weight: Font.DemiBold }
                PropertySlider_redesign { Layout.fillWidth: true; Layout.preferredHeight: 36; label: "Opacity"; from: 0; to: 100; defaultValue: 100; value: root.selectedAsset ? root.selectedAsset.opacity ?? 100 : 100; suffix: "%"; enabled: root.selectedAsset !== null; onEdited: value => root.updateSelected("opacity", value) }
                PropertySlider_redesign { Layout.fillWidth: true; Layout.preferredHeight: 36; label: "Rotation"; from: -180; to: 180; defaultValue: 0; value: root.selectedAsset ? root.selectedAsset.rotation ?? 0 : 0; suffix: "°"; enabled: root.selectedAsset !== null; onEdited: value => root.updateSelected("rotation", value) }
                RowLayout {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 32
                    Toggle_redesign { Layout.fillWidth: true; text: "Mirror"; enabled: root.selectedAsset !== null; checked: root.selectedAsset ? !!root.selectedAsset.flipX : false; onToggled: root.updateSelected("flipX", checked); palette.text: "#C0BDCF" }
                    Toggle_redesign { Layout.fillWidth: true; text: "Smooth scaling"; enabled: root.selectedAsset !== null; checked: root.selectedAsset ? root.selectedAsset.smoothing !== false : true; onToggled: root.updateSelected("smoothing", checked); palette.text: "#C0BDCF" }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumWidth: 0
            radius: 10
            color: "#1A1822"
            border.color: "#383341"
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 4
                Text { text: "Playback and sounds"; color: "#F4F2FA"; font.pixelSize: 14; font.weight: Font.DemiBold }
                RowLayout {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 32
                    Toggle_redesign { Layout.fillWidth: true; text: "Play"; enabled: root.selectedAsset !== null; checked: root.selectedAsset ? root.selectedAsset.playing !== false : false; onToggled: root.updateSelected("playing", checked); palette.text: "#C0BDCF" }
                    Toggle_redesign { Layout.fillWidth: true; text: "Loop"; enabled: root.selectedAsset !== null; checked: root.selectedAsset ? root.selectedAsset.loop !== false : false; onToggled: root.updateSelected("loop", checked); palette.text: "#C0BDCF" }
                }
                RowLayout {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 32
                    Toggle_redesign { Layout.fillWidth: true; text: "Enable sounds"; enabled: root.selectedAsset !== null; checked: root.selectedAsset && root.selectedAsset.audio ? !!root.selectedAsset.audio.enabled : false; onToggled: root.updateAudioEnabled(checked); palette.text: "#C0BDCF" }
                    Button_redesign {
                        text: "Add sound"
                        enabled: root.selectedAsset !== null
                        Layout.preferredWidth: 100
                        Layout.preferredHeight: 32
                        onClicked: root.addAudioRequested()
                        background: Rectangle { radius: 8; color: "#16141F"; border.color: "#383341" }
                        contentItem: Text { text: parent.text; color: parent.enabled ? "#7872C9" : "#777185"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 32
                    ComboBox_redesign {
                        id: actorTrackSelector
                        Layout.fillWidth: true
                        Layout.preferredHeight: 32
                        enabled: root.selectedAsset !== null && root.selectedAsset.audio && root.selectedAsset.audio.files && root.selectedAsset.audio.files.length > 0
                        model: root.selectedAsset && root.selectedAsset.audio ? (root.selectedAsset.audio.files || []).map(file => file.name || String(file.url || "Audio")) : []
                        contentItem: Text { leftPadding: 10; rightPadding: 24; text: parent.displayText || "No audio tracks"; color: parent.enabled ? "#F4F2FA" : "#777185"; font.pixelSize: 12; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
                        background: Rectangle { radius: 8; color: "#16141F"; border.color: "#383341" }
                    }
                    Button_redesign {
                        text: "Configure"
                        enabled: actorTrackSelector.enabled
                        Layout.preferredWidth: 124
                        Layout.preferredHeight: 32
                        onClicked: root.soundSettingsRequested(actorTrackSelector.currentIndex)
                        background: Rectangle { radius: 8; color: "#16141F"; border.color: "#383341" }
                        contentItem: Text { text: parent.text; color: parent.enabled ? "#C0BDCF" : "#777185"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }
                }
                PropertySlider_redesign { Layout.fillWidth: true; Layout.preferredHeight: 36; label: "Speed"; from: 25; to: 200; stepSize: 5; defaultValue: 100; value: root.selectedAsset ? root.selectedAsset.speed ?? 100 : 100; suffix: "%"; enabled: root.selectedAsset !== null; onEdited: value => root.updateSelected("speed", value) }
                PropertySlider_redesign { Layout.fillWidth: true; Layout.preferredHeight: 36; label: "Volume"; from: 0; to: 100; defaultValue: 50; value: root.selectedAsset ? root.selectedAsset.volume ?? 50 : 50; suffix: "%"; enabled: root.selectedAsset !== null; onEdited: value => root.updateSelected("volume", value) }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.preferredHeight: 28
        Text { text: "Your characters"; color: "#F4F2FA"; font.pixelSize: 17; font.weight: Font.DemiBold }
        Rectangle { Layout.preferredWidth: 32; Layout.preferredHeight: 24; radius: 12; color: "#242130"; Text { anchors.centerIn: parent; text: root.assets.length; color: "#C0BDCF"; font.pixelSize: 12 } }
        Item { Layout.fillWidth: true }
        Text { text: "Overlay · Pointer and keyboard input passes through"; color: "#9993AB"; font.pixelSize: 12 }
    }

    ListView {
        id: characterGallery
        objectName: "characterGallery"
        Layout.fillWidth: true
        Layout.preferredHeight: 180
        Layout.minimumHeight: 152
        orientation: ListView.Horizontal
        spacing: 16
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        model: root.assets
        ScrollBar.horizontal: ScrollBar { policy: ScrollBar.AsNeeded }

        delegate: Rectangle {
            id: actorCard
            required property var modelData
            required property int index
            width: Math.max(220, (characterGallery.width - 64) / 5)
            height: 164
            radius: 10
            color: "#1A1822"
            border.width: root.selectedAsset && root.selectedAsset.url === modelData.url ? 2 : 1
            border.color: root.selectedAsset && root.selectedAsset.url === modelData.url ? "#7872C9" : "#383341"

            Image {
                sourceSize: Qt.size(Math.max(1,Math.ceil(width*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(height*(root.Screen.devicePixelRatio || 1))))
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 112
                anchors.margins: 1
                source: actorCard.modelData.thumbnailUrl || actorCard.modelData.url
                fillMode: Image.PreserveAspectFit
                asynchronous: true
            }
            MouseArea { anchors.fill: parent; anchors.bottomMargin: 44; onClicked: root.selected(actorCard.modelData.url) }
            RowLayout {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 44
                anchors.leftMargin: 12
                anchors.rightMargin: 8
                spacing: 8
                Text { Layout.fillWidth: true; text: actorCard.modelData.name; color: "#F4F2FA"; font.pixelSize: 14; elide: Text.ElideRight }
                Text { visible: root.activeUrls.indexOf(actorCard.modelData.url) >= 0; text: "● Ativa"; color: "#34D399"; font.pixelSize: 12 }
                Button_redesign {
                    visible: root.activeUrls.indexOf(actorCard.modelData.url) < 0
                    text: "Enable"
                    Layout.preferredWidth: 92
                    Layout.preferredHeight: 32
                    onClicked: { root.selected(actorCard.modelData.url); root.applyRequested() }
                    background: Rectangle { radius: 8; color: "#242130"; border.color: "#383341" }
                    contentItem: Text { text: parent.text; color: "#7872C9"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }
            }
        }
    }

    function updateSelected(key, value) {
        if (root.selectedAsset)
            root.updateRequested(root.selectedAsset.url, key, value)
    }

    function updateAudioEnabled(enabled) {
        if (!root.selectedAsset)
            return
        root.audioEnabledRequested(enabled)
    }

    property string backgroundUrl: ""
    property string searchText: ""
    property real screenWidth: 1920
    property real screenHeight: 1080
}
