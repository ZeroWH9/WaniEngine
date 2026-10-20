import QtQuick
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root
    WallpaperContent {
        anchors.fill: parent
        configuration: root.configuration
    }
}
