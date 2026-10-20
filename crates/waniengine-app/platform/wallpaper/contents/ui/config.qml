import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    ColumnLayout {
        width: parent.width
        spacing: 12
        Label { text: "WaniEngine"; font.bold: true; font.pixelSize: 18 }
        Label {
            Layout.fillWidth: true
            text: "Choose wallpapers and characters in the WaniEngine library. Content stays active while the application is running. Use Exit in the system tray to restore the previous wallpaper."
            wrapMode: Text.WordWrap
        }
    }
}
