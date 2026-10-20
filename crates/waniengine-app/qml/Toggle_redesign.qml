import QtQuick
import QtQuick.Controls

Switch {
    id: control
    implicitHeight: 40
    implicitWidth: Math.max(44, label.implicitWidth + (text.length ? 56 : 0))
    padding: 0
    hoverEnabled: true
    opacity: enabled ? 1 : 0.45
    indicator: Rectangle {
        x: control.width - width
        y: (control.height - height) / 2
        width: 40
        height: 24
        radius: 12
        color: control.checked ? "#7872C9" : "#373542"
        border.color: control.activeFocus ? "#B0A9E8" : "transparent"
        Rectangle {
            x: control.checked ? 19 : 3
            y: 3
            width: 18
            height: 18
            radius: 9
            color: "#F4F2FA"
            Behavior on x { NumberAnimation { duration: 120 } }
        }
    }
    contentItem: Text {
        id: label
        text: control.text
        rightPadding: control.text.length ? 52 : 0
        color: "#C0BDCF"
        font.pixelSize: 13
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
}
