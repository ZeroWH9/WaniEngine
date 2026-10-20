import QtQuick
import QtQuick.Controls
Button {
    id: control
    property bool primary: false
    implicitHeight: 40
    implicitWidth: Math.max(80, contentItem.implicitWidth + 32)
    padding: 12
    horizontalPadding: 16
    font.pixelSize: 13
    hoverEnabled: true
    opacity: enabled ? 1 : 0.45
    background: Rectangle {
        radius: 10
        color: control.primary ? (control.down ? "#6761B6" : control.hovered ? "#8983D5" : "#7872C9") : (control.down ? "#2D293C" : control.hovered ? "#272433" : "#1E1C28")
        border.color: control.activeFocus ? "#B0A9E8" : control.primary ? "transparent" : "#373341"
    }
    contentItem: Text {
        text: control.text
        color: "#F4F2FA"
        font: control.font
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
}
