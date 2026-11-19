// SPDX-License-Identifier: BSD-3-Clause
// Presentation glue only. Decoding, playback and lifecycle belong to Rust.
import QtQuick
import QtQuick.Window
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root
    property var view: Window.window
    property color savedColor: "black"
    property bool attached: false
    property bool alive: false
    property bool completed: false
    property double lastReply: 0
    property var request: null
    property double requestStarted: 0
    property string acknowledgedEndpoint: ""
    readonly property string endpoint: String(configuration.Endpoint || "")
    readonly property string token: String(configuration.SessionToken || "")
    readonly property bool showingVideo: alive && !!configuration.BridgeReady

    function syncColor() {
        if (view) {
            view.color = showingVideo ? "transparent" : savedColor;
        }
        if (view && showingVideo && acknowledgedEndpoint !== endpoint) {
            acknowledgedEndpoint = endpoint;
            // Confirm this QML item loaded and made its desktop transparent.
            // This is only presentation glue; no playback or commands here.
            Qt.callLater(function() {
                const ack = new XMLHttpRequest();
                ack.open("GET", root.endpoint + "/visible");
                ack.send();
            });
        }
    }
    function resetProbe() {
        if (request) { const old = request; request = null; old.abort(); }
        alive = false;
        acknowledgedEndpoint = "";
        lastReply = 0;
        if (completed) probe();
    }
    function probe() {
        if (Date.now() - lastReply > 1000) alive = false;
        if (!/^http:\/\/127\.0\.0\.1:[0-9]+\/[a-zA-Z0-9-]+$/.test(endpoint)) {
            alive = false;
            return;
        }
        if (request) {
            if (Date.now() - requestStarted <= 1000) return;
            request.abort();
            request = null;
        }
        const expectedToken = token;
        const expectedEndpoint = endpoint;
        const xhr = new XMLHttpRequest();
        request = xhr;
        requestStarted = Date.now();
        xhr.onreadystatechange = function() {
            if (xhr.readyState !== XMLHttpRequest.DONE) return;
            if (root.request === xhr) root.request = null;
            if (root.endpoint !== expectedEndpoint || root.token !== expectedToken) return;
            let healthy = false;
            try {
                const status = JSON.parse(xhr.responseText);
                healthy = xhr.status === 200 && status.ready === true && status.token === expectedToken;
            } catch (error) {}
            root.lastReply = Date.now();
            root.alive = healthy;
            root.syncColor();
        };
        xhr.open("GET", endpoint);
        xhr.send();
    }
    onViewChanged: {
        if (view && !attached) { savedColor = view.color; attached = true; }
        syncColor();
    }
    onShowingVideoChanged: syncColor()
    onEndpointChanged: resetProbe()
    onTokenChanged: resetProbe()
    Timer { interval: 500; repeat: true; running: true; onTriggered: root.probe() }
    Rectangle { anchors.fill: parent; color: "#0c0a11"; visible: !root.showingVideo }
    Image {
        anchors.fill: parent
        visible: !root.showingVideo
        asynchronous: true
        fillMode: Image.PreserveAspectCrop
        source: root.showingVideo ? "" : String(root.configuration.FallbackImage || "")
    }
    Component.onCompleted: {
        if (view && !attached) { savedColor = view.color; attached = true; }
        root.loading = false;
        completed = true;
        probe();
    }
    Component.onDestruction: {
        if (request) request.abort();
        if (view && attached) view.color = savedColor;
    }
}
