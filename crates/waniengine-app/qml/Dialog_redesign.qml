import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
Dialog {
    id: dialog
    parent: Overlay.overlay
    anchors.centerIn: parent
    modal: true
    padding: 24
    topPadding: 12
    bottomPadding: 12
    background: Rectangle { radius: 18; color: "#191722"; border.color: "#393444" }
    Overlay.modal: Rectangle { color: "#99000000" }
    header: Item {
        implicitHeight: 68
        Text { anchors.left: parent.left; anchors.leftMargin: 24; anchors.verticalCenter: parent.verticalCenter; text: dialog.title; color: "#F4F2FA"; font.pixelSize: 22; font.weight: Font.DemiBold }
        Button_redesign { anchors.right: parent.right; anchors.rightMargin: 16; anchors.verticalCenter: parent.verticalCenter; width: 36; height: 36; implicitWidth: 36; text: "×"; onClicked: dialog.reject(); background: Rectangle { radius: 10; color: parent.hovered ? "#302B3D" : "transparent" } }
    }
    footer: DialogButtonBox {
        visible: dialog.standardButtons !== Dialog.NoButton
        standardButtons: dialog.standardButtons
        spacing: 12
        padding: 20
        alignment: Qt.AlignRight
        background: Rectangle { color: "transparent" }
        delegate: Button_redesign { primary: DialogButtonBox.buttonRole === DialogButtonBox.AcceptRole || DialogButtonBox.buttonRole === DialogButtonBox.ApplyRole }
    }
}
