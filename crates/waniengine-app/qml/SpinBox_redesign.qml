import QtQuick
import QtQuick.Controls

SpinBox {
    id: control
    implicitWidth: 140
    implicitHeight: 40
    font.pixelSize: 13
    padding: 0
    leftPadding: 40
    rightPadding: 40
    background: Rectangle {
        color: "#16141F"
        radius: 10
        border.color: control.activeFocus ? "#928BD6" : "#383341"
    }
    contentItem: TextInput {
        text: control.textFromValue(control.value, control.locale)
        color: control.enabled ? "#F4F2FA" : "#777185"
        font: control.font
        horizontalAlignment: Qt.AlignHCenter
        verticalAlignment: Qt.AlignVCenter
        readOnly: !control.editable
        validator: control.validator
        inputMethodHints: Qt.ImhFormattedNumbersOnly
        selectByMouse: true
    }
    up.indicator: Rectangle {
        x: control.width - width
        width: 40
        height: control.height
        radius: 10
        color: control.up.pressed ? "#383341" : control.up.hovered ? "#2C2836" : "transparent"
        Text { anchors.centerIn: parent; text: "+"; font.pixelSize: 22; color: control.up.enabled ? "#C0BDCF" : "#777185" }
    }
    down.indicator: Rectangle {
        width: 40
        height: control.height
        radius: 10
        color: control.down.pressed ? "#383341" : control.down.hovered ? "#2C2836" : "transparent"
        Text { anchors.centerIn: parent; text: "−"; font.pixelSize: 22; color: control.down.enabled ? "#C0BDCF" : "#777185" }
    }
}
