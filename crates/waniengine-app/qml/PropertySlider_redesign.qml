import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

RowLayout {
    id: row
    property string label: ""
    property real from: 0
    property real to: 100
    property real stepSize: 1
    property real defaultValue: 0
    property real value: defaultValue
    property string suffix: ""
    signal edited(real value)

    spacing: 8

    Text {
        text: row.label
        color: "#C0BDCF"
        font.pixelSize: 12
        Layout.preferredWidth: 112
        elide: Text.ElideRight
    }

    Slider {
        id: slider
        objectName: "slider"
        Layout.fillWidth: true
        Layout.preferredHeight: 40
        from: row.from
        to: row.to
        stepSize: row.stepSize
        value: row.value
        onMoved: row.edited(Math.round(value / stepSize) * stepSize)
        Accessible.name: row.label
    }

    Text {
        text: Math.round(row.value) + row.suffix
        color: "#F4F2FA"
        font.pixelSize: 12
        Layout.preferredWidth: 48
        horizontalAlignment: Text.AlignRight
    }

    Button_redesign {
        objectName: "resetButton"
        Layout.preferredWidth: 32
        Layout.preferredHeight: 32
        hoverEnabled: true
        Accessible.name: "Reset " + row.label + " to " + row.defaultValue + row.suffix
        ToolTip.visible: hovered
        ToolTip.delay: 350
        ToolTip.text: "Reset: " + row.defaultValue + row.suffix
        background: Rectangle {
            radius: 8
            color: parent.down ? "#302B43" : parent.hovered ? "#292535" : "transparent"
            border.color: parent.hovered ? "#7872C9" : "#383341"
        }
        contentItem: Text {
            text: "↺"
            color: "#C0BDCF"
            font.pixelSize: 16
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        onClicked: row.edited(row.defaultValue)
    }
}
