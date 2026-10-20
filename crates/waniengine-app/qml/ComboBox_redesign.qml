import QtQuick
import QtQuick.Controls
ComboBox {
    id: control
    implicitHeight: 40
    implicitWidth: 180
    font.pixelSize: 13
    opacity: enabled ? 1 : 0.45
    contentItem: Text { leftPadding: 12; rightPadding: 32; text: control.displayText; color: "#F4F2FA"; font: control.font; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
    indicator: Text { x: control.width - 25; y: (control.height-height)/2; text: "⌄"; color: "#AAA6BC"; font.pixelSize: 18 }
    background: Rectangle { radius: 10; color: "#16141F"; border.color: control.activeFocus ? "#8881CC" : "#373341" }
    delegate: ItemDelegate { width: control.width; text: control.textRole ? modelData[control.textRole] : modelData; highlighted: control.highlightedIndex === index; palette.text: "#F4F2FA"; palette.highlight: "#353046"; font.pixelSize: 13 }
    popup: Popup {
        y: control.height + 6
        width: control.width
        padding: 6
        implicitHeight: Math.min(320, contentItem.implicitHeight + 12)
        background: Rectangle { radius: 12; color: "#211E2B"; border.color: "#373341" }
        contentItem: ListView { clip: true; implicitHeight: contentHeight; model: control.popup.visible ? control.delegateModel : null; currentIndex: control.highlightedIndex; ScrollIndicator.vertical: ScrollIndicator {} }
    }
}
