import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtMultimedia
import WaniEngine.Runtime 1.0 as Engine

ApplicationWindow {
    id: root

    width: 1280
    height: 800
    minimumWidth: 1100
    minimumHeight: 700
    visible: !startHidden
    title: "WaniEngine"
    color: "#100F16"
    palette.text: "#F4F2FA"
    palette.windowText: "#F4F2FA"
    palette.buttonText: "#F4F2FA"
    palette.window: "#100F16"
    palette.base: "#16141F"
    palette.button: "#242130"
    palette.highlight: "#7872C9"

    readonly property color surface: "#100F16"
    readonly property color panel: "#1A1822"
    readonly property color field: "#16141F"
    readonly property color control: "#242130"
    readonly property color border: "#383341"
    readonly property color divider: "#2C2836"
    readonly property color textMain: "#F4F2FA"
    readonly property color textMuted: "#C0BDCF"
    readonly property color textDisabled: "#777185"
    readonly property color blue: "#7872C9"
    readonly property color blueHover: "#A19BDC"
    readonly property color pink: "#FB7185"

    property var assets: []
    property var playlist: []
    property string playlistName: "Unsaved"
    property bool playlistDirty: false
    property bool playlistRunning: false
    property string selectedUrl: ""
    property string selectedTab: "installed"
    property string contentFilter: "all"
    property string searchText: ""
    property string sortMode: "recent"
    property bool filterOpen: false
    property bool favoritesOnly: false
    property bool showWallpaperAdjustments: false
    property bool busyApplying: false
    property var pendingApplication: null
    property string previewUrl: ""
    property int previewRequest: 0
    property bool preparingPreview: false
    property bool animationPreviewPaused: false
    property string selectedMonitor: ""
    property var wallpapersByMonitor: ({})
    property var restoredWallpapers: ({})
    property var restoreAttemptsByMonitor: ({})
    property var restoreGiveUp: ({})
    property bool restoreRequested: false
    property bool restoreBusy: false
    property bool restoredActors: false
    readonly property var monitorNames: plasmaIntegration.monitors.map((monitor) => monitor.name)
    readonly property var monitorLabels: plasmaIntegration.monitors.map((monitor) => monitor.name + "  ·  " + monitor.width + " × " + monitor.height)
    readonly property var selectedScreen: plasmaIntegration.monitors.find(item => item.name === selectedMonitor) || plasmaIntegration.monitors[0] || ({width:width,height:height,refreshRate:60,scale:1})
    readonly property int maxFps: Math.max(15,Math.round(selectedScreen.refreshRate || 60))
    readonly property int effectiveFps: Math.max(15,Math.min(maxFps,Number(settings.fpsLimit || 60)))
    readonly property int renderPixelWidth: Math.max(2,selectedScreen.pixelWidth || Math.round(selectedScreen.width*(selectedScreen.scale || 1)))
    readonly property int renderPixelHeight: Math.max(2,selectedScreen.pixelHeight || Math.round(selectedScreen.height*(selectedScreen.scale || 1)))
    readonly property var sampleLevels: typeof sceneStore.sampleLevels === "function" ? sceneStore.sampleLevels() : [0]
    property string statusMessage: ""
    property string backgroundUrl: ""
    property string backgroundPlaybackUrl: ""
    property var settings: ({
        pauseFullscreen: true,
        pauseMaximized: false,
        startWithSession: false,
        restoreOnLaunch: true,
        closeToTray: false,
        animatePreview: true,
        fpsLimit: 60,
        samples: 0,
        globalVolume: 100,
        audioOutputId: ""
    })

    readonly property real inspectorWidth: Math.max(280, Math.min(380, width * 0.2))
    readonly property var selectedAsset: {
        for (let i = 0; i < assets.length; ++i) {
            if (assets[i].url === selectedUrl)
                return assets[i]
        }
        return null
    }
    readonly property var visibleAssets: {
        const needle = searchText.trim().toLowerCase()
        let rows = assets.filter((item) => {
            const matchesText = needle.length === 0 || item.name.toLowerCase().includes(needle)
            const matchesKind = contentFilter === "all" || item.kind === contentFilter
            const matchesFavorite = !favoritesOnly || item.favorite
            const matchesTab = selectedTab !== "animations" || item.kind === "character"
            return matchesText && matchesKind && matchesFavorite && matchesTab
        })
        if (sortMode === "name")
            rows.sort((a, b) => a.name.localeCompare(b.name))
        else if (sortMode === "type")
            rows.sort((a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name))
        else
            rows.sort((a, b) => (b.addedAt || 0) - (a.addedAt || 0))
        return rows
    }

    function fileName(url) {
        const clean = decodeURIComponent(String(url).split(/[?#]/)[0])
        return clean.substring(clean.lastIndexOf("/") + 1) || "Unnamed file"
    }

    function fileExtension(url) {
        const name = fileName(url).toLowerCase()
        const dot = name.lastIndexOf(".")
        return dot >= 0 ? name.substring(dot + 1) : ""
    }

    function isVideoFile(url) {
        return ["mp4", "mkv", "webm", "mov", "avi", "m4v"].indexOf(fileExtension(url)) >= 0
    }

    function isSupportedFile(url) {
        return isVideoFile(url)
            || ["png", "jpg", "jpeg", "webp", "gif", "apng", "bmp", "svg"].indexOf(fileExtension(url)) >= 0
    }

    function importFiles(files, kind) {
        let next = assets.slice(0)
        let firstAdded = ""
        for (let i = 0; i < files.length; ++i) {
            const url = String(files[i])
            if (!isSupportedFile(url))
                continue
            const existing = next.findIndex((item) => item.url === url || item.originalSource === url)
            if (existing >= 0)
                continue
            const extension = fileExtension(url)
            const inferredKind = kind || ((["gif", "apng"].indexOf(extension) >= 0)
                                          ? "character" : "wallpaper")
            let item = {
                url: url,
                name: fileName(url),
                extension: extension.toUpperCase(),
                kind: inferredKind,
                video: isVideoFile(url),
                favorite: false,
                addedAt: Date.now(),
                speed: 100,
                posX: 50, posY: 96, opacity: 100, flipX: false,
                scale: 100,
                volume: 50,
                muted: true,
                fit: "cover"
            }
            if(typeof sceneStore.describeMedia === "function") Object.assign(item,JSON.parse(sceneStore.describeMedia(url)))
            if (typeof sceneStore.organizeAsset === "function") {
                item = JSON.parse(sceneStore.organizeAsset(JSON.stringify(item)))
                if (!item.url) {showStatus("Could not add this file to the library.");continue}
            }
            next.push(item)
            if (firstAdded.length === 0)
                firstAdded = item.url
        }
        assets = next
        if (firstAdded.length > 0) {
            selectedTab = kind === "character" ? "animations" : "installed"
            contentFilter = "all"
            selectedUrl = firstAdded
        }
        saveState()
        if (firstAdded.length > 0)
            showStatus("Content added to the library.")
        else
            showStatus("No supported files were added.")
    }

    function updateAsset(url, propertyName, value) {
        const next = assets.slice(0)
        const index = next.findIndex((item) => item.url === url)
        if (index < 0)
            return
        let updated = Object.assign({}, next[index])
        updated[propertyName] = value
        next[index] = updated
        assets = next
        saveState()
        if (propertyName !== "favorite" && !livePropertyTimer.running) livePropertyTimer.start()
    }

    function wallpaperEntry(monitor) {
        return wallpapersByMonitor[monitor] || ({})
    }

    function selectMonitor(monitor, persist) {
        if (!monitor) return
        selectedMonitor = monitor
        const entry = wallpaperEntry(monitor)
        backgroundUrl = entry.url || ""
        backgroundPlaybackUrl = entry.playbackUrl || ""
        if (entry.url && assets.some(item => item.url === entry.url)) selectedUrl = entry.url
        if (persist !== false) saveState()
    }

    function reconcileSelectedMonitor() {
        if (!monitorNames.length) return
        if (!selectedMonitor) {
            selectMonitor(monitorNames[0],false)
        } else if (monitorNames.indexOf(selectedMonitor) >= 0) {
            const entry = wallpaperEntry(selectedMonitor)
            backgroundUrl = entry.url || ""
            backgroundPlaybackUrl = entry.playbackUrl || ""
        } else if (!wallpapersByMonitor[selectedMonitor]) {
            selectMonitor(monitorNames[0],false)
        }
    }

    function toggleFavorite(url) {
        const item = assets.find((entry) => entry.url === url)
        if (item)
            updateAsset(url, "favorite", !item.favorite)
    }

    function addSelectedToPlaylist() {
        if (!selectedAsset || selectedAsset.kind !== "wallpaper") {
            showStatus("Playlists rotate wallpapers. Enable characters in Animations.")
            return
        }
        if (playlist.indexOf(selectedAsset.url) >= 0) {
            showStatus("This wallpaper is already in the playlist.")
            return
        }
        playlist = playlist.concat([selectedAsset.url])
        playlistDirty = true
        saveState()
        showStatus("Added to playlist.")
    }

    function removeFromPlaylist(url) {
        playlist = playlist.filter(item => item !== url)
        playlistDirty = true
        if (!playlist.length) playlistRunning = false
        saveState()
        showStatus("Removed from playlist. The content remains in your library.")
    }
    function savePlaylistFile(url) {
        const items=playlist.map(url => assets.find(item => item.url===url)).filter(item => !!item && item.kind==="wallpaper")
        const name=fileName(url).replace(/\.json$/i, "")
        const document={format:"waniengine-playlist",version:1,name:name,assets:items,
            intervalMinutes:settings.playlistInterval || 30,random:!!settings.playlistRandom}
        if (!playlistStore.save(url,JSON.stringify(document))) {showStatus("Could not save the playlist.");return}
        playlistName=name;playlistDirty=false;saveState()
        showStatus("Playlist saved to " + decodeURIComponent(String(url).replace("file://", "")))
    }
    function loadPlaylistFile(url) {
        const json=playlistStore.load(url)
        if (!json) {showStatus("The playlist is invalid or unavailable.");return}
        const document=JSON.parse(json)
        let next=assets.slice(0), urls=[], unavailable=0
        for (const item of document.assets) {
            const metadata=typeof sceneStore.describeMedia === "function" ? JSON.parse(sceneStore.describeMedia(item.url)) : {width:1}
            if (!metadata.width) {++unavailable;continue}
            let resolved=next.find(entry => entry.url===item.url || entry.originalSource===item.url || (!!entry.id && entry.id===item.id))
            if (!resolved) {
                resolved=Object.assign({},item,metadata)
                if (typeof sceneStore.organizeAsset === "function") resolved=JSON.parse(sceneStore.organizeAsset(JSON.stringify(resolved)))
                if (!resolved.url) {++unavailable;continue}
                next.push(resolved)
            }
            if (urls.indexOf(resolved.url)<0) urls.push(resolved.url)
        }
        playlistRunning=false;assets=next;playlist=urls;playlistName=document.name || fileName(url);playlistDirty=false
        settings=Object.assign({},settings,{playlistInterval:Math.max(1,Math.min(1440,Number(document.intervalMinutes || 30))),playlistRandom:!!document.random})
        saveState()
        showStatus("Playlist loaded: " + playlistName + (unavailable ? " — " + unavailable + " missing file(s)." : "."))
    }
    function nextPlaylistWallpaper() {
        if(busyApplying)return
        const urls=playlist.filter(url=>assets.some(item=>item.url===url && item.kind==="wallpaper"))
        if (!urls.length) {playlistRunning=false;return}
        const current=urls.indexOf(backgroundUrl)
        let index=(current+1)%urls.length
        if (settings.playlistRandom && urls.length>1) {
            const choices=urls.filter(url=>url!==backgroundUrl)
            index=urls.indexOf(choices[Math.floor(Math.random()*choices.length)])
        }
        const selection=selectedUrl
        selectedUrl=urls[index];applySelected();selectedUrl=selection;saveState()
        if (!busyApplying && (!plasmaIntegration.active || backgroundUrl!==urls[index])) playlistRunning=false
    }
    function togglePlaylist() {
        if (playlistRunning) {playlistRunning=false;showStatus("Playlist paused. Your current wallpaper stays applied.");return}
        playlistRunning=true
        if (playlist.indexOf(backgroundUrl)<0) nextPlaylistWallpaper()
    }
    Timer {
        interval: Math.max(1,Number(root.settings.playlistInterval || 30))*60000
        repeat: true
        running: root.playlistRunning && root.playlist.length>0
        onTriggered: root.nextPlaylistWallpaper()
    }
    QtObject {
        id: playlistLoadFiles
        function open() { const file = nativeFilePicker.openFile("Load playlist", String(playlistStore.directory()), ["WaniEngine playlist (*.json)"]); if (String(file).length) { root.loadPlaylistFile(String(file)) } }
    }
    QtObject {
        id: playlistSaveFiles
        function open() { const file = nativeFilePicker.saveFile("Save playlist as", String(playlistStore.directory()), ["WaniEngine playlist (*.json)"]); if (String(file).length) { root.savePlaylistFile(String(file)) } }
    }

    function removeAsset(url) {
        if(actorUrls.indexOf(url)>=0) {
            const next=actorUrls.filter((entry)=>entry!==url)
            if(!overlayController.applyActors(actorsConfiguration(next),selectedMonitor)) {showStatus(overlayController.lastError);return}
            actorUrls=next
        }
        assets = assets.filter((item) => item.url !== url)
        removeFromPlaylist(url)
        if (selectedUrl === url)
            selectedUrl = assets.length > 0 ? assets[0].url : ""
        if (backgroundUrl === url)
            backgroundUrl = ""
        saveState()
        showStatus("Removed from the library. The original file was preserved.")
        removeDialog.close()
    }

    function actorsConfiguration(urls) {
        const entries = []
        for (let i = 0; i < urls.length; ++i) {
            const asset = assets.find((item) => item.url === urls[i])
            if (!asset)
                continue
            entries.push({
                url: asset.url,
                x: Number(asset.posX === undefined ? (i + 1) * 100 / (urls.length + 1) : asset.posX) / 100,
                y: Number(asset.posY === undefined ? 96 : asset.posY) / 100,
                scale: Number(asset.scale || 100),
                speed: Number(asset.speed || 100),
                opacity: Number(asset.opacity === undefined ? 100 : asset.opacity),
                flipX: !!asset.flipX, playing: asset.playing !== false && !automaticPause, loop: asset.loop !== false,
                rotation: Number(asset.rotation || 0), smoothing: asset.smoothing !== false,
                fpsLimit: effectiveFps, samples: settings.samples || 0, width:asset.width || 1,height:asset.height || 1,
                volume: asset.volume ?? 50,audio: asset.audio || ({enabled:false,files:[]})
            })
        }
        return JSON.stringify(entries)
    }

    MediaDevices { id: audioDevices }
    function selectedAudioDevice(id) {
        for (let i=0; i<audioDevices.audioOutputs.length; ++i)
            if (String(audioDevices.audioOutputs[i].id) === String(id || "")) return audioDevices.audioOutputs[i]
        return audioDevices.defaultAudioOutput
    }
    function audioConfiguration() {
        return JSON.stringify({ deviceId: settings.audioOutputId || "", globalVolume: settings.globalVolume ?? 100,managerActive: applicationController.managerActive === true })
    }

    function visualConfiguration(asset) {
        return JSON.stringify({zoom:asset.zoom ?? 100,offsetX:asset.offsetX ?? 0,offsetY:asset.offsetY ?? 0,
            flipX:!!asset.flipX,smoothing:asset.smoothing !== false,motion:asset.motion || "none",
            motionAmount:asset.motionAmount ?? 2,motionPeriod:asset.motionPeriod ?? 12,fpsLimit:effectiveFps,samples:settings.samples || 0})
    }
    Timer {
        id: livePropertyTimer
        interval: 60
        onTriggered: {
            root.syncActorAudio()
            if (overlayController.active && root.actorUrls.length) {
                if (!overlayController.applyActors(root.actorsConfiguration(root.actorUrls),root.selectedMonitor)) root.showStatus(overlayController.lastError)
            }
            const asset=root.assets.find(item=>item.url===root.backgroundUrl && item.kind==="wallpaper")
            if (plasmaIntegration.active && asset) {
                const properties={FitMode:asset.fit || "cover",Speed:Number(asset.speed ?? 100),Volume:Number(asset.volume ?? 50),
                    AudioEnabled:!!asset.audioEnabled,Playing:asset.playing !== false && !root.automaticPause,Loop:asset.loop !== false,
                    VisualOptionsJson:root.visualConfiguration(asset)}
                if (!plasmaIntegration.updateContentOptions(root.backgroundPlaybackUrl || asset.url,root.selectedMonitor,JSON.stringify(properties))) root.showStatus(plasmaIntegration.lastError)
            }
        }
    }

    function syncActorAudio() {
        if (typeof actorAudioController !== "undefined") actorAudioController.configure(automaticPause ? "[]" : actorsConfiguration(actorUrls),audioConfiguration(),selectedMonitor)
    }
    function updateAudio(key,value) {
        if (!selectedAsset) return
        const audio=Object.assign({enabled:false,mode:"loop",intervalSeconds:30,cooldownSeconds:2,radiusPixels:40,random:false,files:[]},selectedAsset.audio || ({}))
        audio[key]=value;updateAsset(selectedUrl,"audio",audio)
    }
    function addAudioFiles(files) {
        if (!selectedAsset || typeof sceneStore.addAudioFiles !== "function") return
        const tracks=JSON.parse(sceneStore.addAudioFiles(JSON.stringify(selectedAsset),JSON.stringify(files.map(item => String(item)))))
        if(sceneStore.lastError) {showStatus(sceneStore.lastError);return}
        updateAudio("files",tracks)
        showStatus("Audio files added to this character.")
    }
    function trackSettings(index) {
        const audio=selectedAsset && selectedAsset.audio ? selectedAsset.audio : ({})
        const file=(audio.files || [])[index] || ({})
        return Object.assign({enabled:true,mode:audio.mode || "once",volume:100,intervalSeconds:audio.intervalSeconds ?? 30,
            radiusPixels:audio.radiusPixels ?? 40,cooldownSeconds:audio.cooldownSeconds ?? 2,startImmediately:audio.startImmediately !== false},file.playback || ({}))
    }
    function audioModeLabel(mode) {
        return ({loop:"Continuous",once:"When activated",interval:"At intervals",click:"When clicking the character"})[mode] || "When activated"
    }
    function editTrack(index,key,value) {
        if (!selectedAsset || !selectedAsset.audio) return
        const playback=trackSettings(index);playback[key]=value
        const files=selectedAsset.audio.files.map((item,i)=>i===index ? Object.assign({},item,{playback:playback}) : item)
        updateAudio("files",files)
    }
    Dialog_redesign {
        id:soundSettings;objectName:"soundSettingsDialog"
        width:500;modal:true
        title:"Sound settings";standardButtons:Dialog.NoButton
        footer:DialogButtonBox { spacing:12;padding:20; Button_redesign {text:"Done";onClicked:soundSettings.close()} }
        property int trackIndex:-1
        readonly property var options:root.trackSettings(trackIndex)
        contentItem:ColumnLayout {
            spacing:10
            Text {Layout.fillWidth:true;text:root.selectedAsset && root.selectedAsset.audio && (root.selectedAsset.audio.files || [])[soundSettings.trackIndex] ? root.selectedAsset.audio.files[soundSettings.trackIndex].name : "";color:root.textMain;font.bold:true;font.pixelSize:12;wrapMode:Text.WordWrap}
            Toggle_redesign {text:"Enable sound";palette.text:root.textMain;checked:soundSettings.options.enabled;onToggled:root.editTrack(soundSettings.trackIndex,"enabled",checked)}
            Label {text:"Playback trigger";color:root.textMuted}
            ComboBox_redesign {
                objectName:"actorAudioMode";Layout.fillWidth:true
                model:["When activated","Continuous","At intervals","When clicking the character"]
                currentIndex:Math.max(0,["once","loop","interval","click"].indexOf(soundSettings.options.mode))
                onActivated:index=>root.editTrack(soundSettings.trackIndex,"mode",["once","loop","interval","click"][index])
            }
            RowLayout {
                visible:soundSettings.options.mode === "interval";Layout.fillWidth:true
                Text {text:"Interval in seconds";color:root.textMuted;font.pixelSize:11}
                Item {Layout.fillWidth:true}
                SpinBox_redesign {from:1;to:86400;editable:true;value:soundSettings.options.intervalSeconds;onValueModified:root.editTrack(soundSettings.trackIndex,"intervalSeconds",value)}
            }
            Toggle_redesign {visible:soundSettings.options.mode === "interval";text:"Play immediately on activation";palette.text:root.textMain;checked:soundSettings.options.startImmediately;onToggled:root.editTrack(soundSettings.trackIndex,"startImmediately",checked)}
            PropertySlider_redesign {Layout.fillWidth:true;label:"Volume";from:0;to:100;defaultValue:100;suffix:"%";value:soundSettings.options.volume;onEdited:value=>root.editTrack(soundSettings.trackIndex,"volume",value)}
            PropertySlider_redesign {visible:soundSettings.options.mode === "click";Layout.fillWidth:true;label:"Click radius";from:0;to:500;defaultValue:40;suffix:" px";value:soundSettings.options.radiusPixels;onEdited:value=>root.editTrack(soundSettings.trackIndex,"radiusPixels",value)}
            PropertySlider_redesign {visible:soundSettings.options.mode === "click";Layout.fillWidth:true;label:"Minimum cooldown";from:0;to:60;defaultValue:2;suffix:" s";value:soundSettings.options.cooldownSeconds;onEdited:value=>root.editTrack(soundSettings.trackIndex,"cooldownSeconds",value)}
            Text {visible:soundSettings.options.mode === "click";Layout.fillWidth:true;text:"Works on the desktop with an app wallpaper applied. Clicks inside games do not trigger sounds.";color:"#e8bc65";font.pixelSize:11;wrapMode:Text.WordWrap}
            Button_redesign {text:"Test sound";Layout.fillWidth:true;enabled:root.actorUrls.indexOf(root.selectedUrl)>=0 && root.selectedAsset && !!root.selectedAsset.audio && root.selectedAsset.audio.enabled && soundSettings.options.enabled;onClicked:{const file=root.selectedAsset.audio.files[soundSettings.trackIndex];root.syncActorAudio();actorAudioController.testTrack(root.selectedUrl,file.url)}}
            Text {Layout.fillWidth:true;text:"Changes apply immediately to the active character.";color:root.textMuted;font.pixelSize:10;wrapMode:Text.WordWrap}
        }
    }
    QtObject {
        id: actorAudioFiles
        function open() { const files = nativeFilePicker.openFiles("Add character audio", ["Audio (*.wav *.mp3 *.ogg *.flac *.m4a *.aac *.opus)"]); if (files.length) { root.addAudioFiles(files) } }
    }
    Connections {
        target: applicationController
        function onManagerActiveChanged() { if (!plasmaIntegration.updateAudioOptions(root.audioConfiguration())) root.showStatus(plasmaIntegration.lastError) }
    }
    Connections {
        target: typeof actorAudioController !== "undefined" ? actorAudioController : null
        function onLastErrorChanged() { if (actorAudioController.lastError) root.showStatus(actorAudioController.lastError) }
    }
    onActorUrlsChanged: syncActorAudio()
    onEffectiveFpsChanged: if (!livePropertyTimer.running) livePropertyTimer.start()
    function applySelected() {
        if (!selectedAsset || selectedAsset.missing || busyApplying) return
        if (selectedAsset.kind === "character") {
            if (overlayController.active && actorUrls.indexOf(selectedUrl)>=0) { removeSelectedActor(); return }
            const next=overlayController.active ? actorUrls.slice(0) : []
            if(next.indexOf(selectedUrl)<0) next.push(selectedUrl)
            if(!overlayController.applyActors(actorsConfiguration(next),selectedMonitor)) {
                showStatus(overlayController.lastError); return
            }
            actorUrls=next
            showStatus("Character enabled. Pointer input passes through it. Exit removes the overlay.")
        } else {
            if (selectedAsset.video) {
                pendingApplication={url:selectedUrl,monitor:selectedMonitor}
                busyApplying=true
                mediaOptimizer.prepareVideoAsync(selectedUrl,Number(selectedAsset.width || 0),Number(selectedAsset.height || 0),renderPixelWidth,renderPixelHeight)
                return
            }
            finishWallpaper(selectedAsset,selectedMonitor,selectedUrl,false)

        }
        saveState()
    }

    function finishWallpaper(asset,monitor,playbackUrl,optimized) {
        if(!asset) return false
        if(!plasmaIntegration.applyContent(playbackUrl,"wallpaper","[]",monitor,!!asset.audioEnabled,asset.fit || "cover",Number(asset.speed || 100),Number(asset.volume ?? 50),asset.playing !== false && !automaticPause,asset.loop !== false,visualConfiguration(asset),audioConfiguration())) {showStatus(plasmaIntegration.lastError);return false}
        const nextWallpapers = Object.assign({}, wallpapersByMonitor)
        nextWallpapers[monitor] = {url:asset.url,playbackUrl:playbackUrl}
        wallpapersByMonitor = nextWallpapers
        markWallpaperRestored(monitor,asset.url)
        if(monitor === selectedMonitor) {
            backgroundUrl=asset.url
            backgroundPlaybackUrl=playbackUrl
        }
        showStatus(optimized ? "Wallpaper applied using a display-sized video." : "Wallpaper applied.")
        saveState()
        return true
    }
    Connections {
        target:mediaOptimizer
        function onPreviewPrepared(result) {
            if(result.request !== root.previewRequest)return
            root.preparingPreview=false
            if(!root.visible || root.selectedTab !== "installed" || !root.selectedAsset || root.selectedAsset.url!==result.source)return
            if(result.ok)root.previewUrl=result.url
            else root.showStatus(result.error || "Could not prepare the preview.")
        }
        function onPrepared(result) {
            const request=root.pendingApplication
            root.pendingApplication=null
            root.busyApplying=false
            if(!request)return
            let applied=false
            if(!result.ok){root.playlistRunning=false;root.showStatus(result.error)}
            else {
                const asset=root.assets.find(item=>item.url===request.url)
                applied=root.finishWallpaper(asset,request.monitor,result.url,!!result.optimized)
            }
            if(request.restoring) {
                if(!applied) root.markWallpaperRestoreFailure(request.monitor,request.url)
                root.restoreBusy=false
                if(applied) Qt.callLater(root.restoreSavedWallpapers)
            }
        }
    }
    Connections {target:sceneStore;function onLastErrorChanged(){if(sceneStore.lastError)root.showStatus(sceneStore.lastError)}}
    Connections {target:playlistStore;function onLastErrorChanged(){if(playlistStore.lastError)root.showStatus(playlistStore.lastError)}}

    function navigate(page) {
        if(page === "settings") {settingsDialog.open();return}
        selectedTab=page
        contentFilter="all"
        searchText=""
        const kind=page === "animations" ? "character" : "wallpaper"
        if(!selectedAsset || selectedAsset.kind !== kind) {
            const first=assets.find(asset=>asset.kind===kind && !asset.missing)
            selectedUrl=first ? first.url : ""
        }
    }
    function saveState() {
        const savedWallpapers = ({})
        Object.keys(wallpapersByMonitor).forEach(monitor => {
            const entry = wallpaperEntry(monitor)
            if (entry.url) savedWallpapers[monitor] = {url:entry.url}
        })
        const state = {
            schema: 1,
            assets: assets,
            playlist: playlist,
            playlistName: playlistName,
            playlistDirty: playlistDirty,
            selectedUrl: selectedUrl,
            selectedMonitor: selectedMonitor,
            wallpapersByMonitor: savedWallpapers,
            actorUrls: actorUrls,
            settings: settings
        }
        if (!sceneStore.saveScene(JSON.stringify(state))) showStatus(sceneStore.lastError || "Could not save library state.")
    }

    function restoreState() {
        try {
            const state = JSON.parse(sceneStore.loadScene())
            assets = (state.assets || []).map((item) => {
                const extension = fileExtension(item.url || "")
                return Object.assign({
                    extension: extension.toUpperCase(),
                    kind: item.kind || (item.video ? "wallpaper" : "character"),
                    video: item.video === undefined ? isVideoFile(item.url || "") : item.video,
                    favorite: false,
                    addedAt: 0,
                    fps: 15,
                    scale: 100,
                    volume: 50,
                    muted: true,
                    fit: "cover"
                }, item, !item.width && !item.missing && typeof sceneStore.describeMedia === "function" ? JSON.parse(sceneStore.describeMedia(item.url)) : {})
            })
            playlist = (state.playlist || []).filter(url => assets.some(item => item.url===url && item.kind==="wallpaper"))
            playlistName = state.playlistName || "Unsaved"
            playlistDirty = !!state.playlistDirty
            const initial=assets.find(asset=>asset.url===state.selectedUrl && asset.kind==="wallpaper") || assets.find(asset=>asset.kind==="wallpaper" && !asset.missing)
            selectedUrl=initial ? initial.url : ""
            selectedMonitor = state.selectedMonitor || ""
            const savedWallpapers = ({})
            const persistedWallpapers = state.wallpapersByMonitor || ({})
            Object.keys(persistedWallpapers).forEach(monitor => {
                const entry = persistedWallpapers[monitor]
                if (monitor && entry && entry.url) savedWallpapers[monitor] = {url:entry.url}
            })
            wallpapersByMonitor = savedWallpapers
            restoredWallpapers = ({})
            restoreAttemptsByMonitor = ({})
            restoreGiveUp = ({})
            restoredActors = false
            const currentWallpaper = wallpaperEntry(selectedMonitor)
            backgroundUrl = currentWallpaper.url || ""
            backgroundPlaybackUrl = ""
            restoredActorUrls = (state.actorUrls || []).filter(url=>assets.some(asset=>asset.url===url && asset.kind==="character" && !asset.missing))
            actorUrls = []
            settings = Object.assign(settings, state.settings || {})
        } catch (error) {
            assets = []
            playlist = []
            actorUrls = []
        }
    }

    function reloadLibrary() {
        const previousUrl = selectedUrl
        saveState()
        restoreState()
        if (assets.some(asset => asset.url === previousUrl))
            selectedUrl = previousUrl
        showStatus("Library reloaded.")
    }

    property var actorUrls: []
    property var restoredActorUrls: []
    property bool shuttingDown: false
    property bool automaticPause: false

    function restoreSavedWallpapers() {
        if (!restoreRequested || restoreBusy || busyApplying) return
        for (const monitor of Object.keys(wallpapersByMonitor)) {
            const entry = wallpaperEntry(monitor)
            if (!entry.url || monitorNames.indexOf(monitor) < 0 || restoredWallpapers[monitor] === entry.url || restoreGiveUp[monitor] === entry.url) continue
            const asset = assets.find(item => item.url === entry.url && item.kind === "wallpaper")
            if (!asset || asset.missing) {
                const giveUp = Object.assign({}, restoreGiveUp)
                giveUp[monitor] = entry.url
                restoreGiveUp = giveUp
                continue
            }
            const screen = plasmaIntegration.monitors.find(item => item.name === monitor)
            if (!screen) continue
            restoreBusy = true
            if (asset.video) {
                const width = Number(screen.pixelWidth || Math.round(screen.width * (screen.scale || 1)))
                const height = Number(screen.pixelHeight || Math.round(screen.height * (screen.scale || 1)))
                pendingApplication = {url:asset.url,monitor:monitor,restoring:true}
                busyApplying = true
                mediaOptimizer.prepareVideoAsync(asset.url,Number(asset.width || 0),Number(asset.height || 0),width,height)
                return
            }
            const applied = finishWallpaper(asset,monitor,asset.url,false)
            restoreBusy = false
            if (applied) Qt.callLater(restoreSavedWallpapers)
            else markWallpaperRestoreFailure(monitor,entry.url)
            return
        }
        if (!restoredActors && restoredActorUrls.length && monitorNames.indexOf(selectedMonitor) >= 0) {
            if (overlayController.applyActors(actorsConfiguration(restoredActorUrls),selectedMonitor))
                actorUrls = restoredActorUrls.slice(0)
            else
                showStatus(overlayController.lastError)
            restoredActors = true
        }
        saveState()
    }

    function markWallpaperRestored(monitor,url) {
        const restored=Object.assign({},restoredWallpapers)
        restored[monitor]=url
        restoredWallpapers=restored
        const attempts=Object.assign({},restoreAttemptsByMonitor)
        delete attempts[monitor]
        restoreAttemptsByMonitor=attempts
        const giveUp=Object.assign({},restoreGiveUp)
        delete giveUp[monitor]
        restoreGiveUp=giveUp
    }

    function markWallpaperRestoreFailure(monitor,url) {
        const attempts=Object.assign({},restoreAttemptsByMonitor)
        attempts[monitor]=Number(attempts[monitor] || 0)+1
        restoreAttemptsByMonitor=attempts
        if(attempts[monitor]>=30) {
            const giveUp=Object.assign({},restoreGiveUp)
            giveUp[monitor]=url
            restoreGiveUp=giveUp
            showStatus("Could not restore the wallpaper on " + monitor + ". Apply it again from the library.")
        }
    }

    function hasPendingConnectedWallpapers() {
        if(!restoreRequested) return false
        return Object.keys(wallpapersByMonitor).some(monitor => {
            const url=wallpaperEntry(monitor).url
            return !!url && monitorNames.indexOf(monitor)>=0 && restoredWallpapers[monitor]!==url && restoreGiveUp[monitor]!==url
        })
    }
    function updateWindowPolicy() {
        automaticPause=false
    }
    onAutomaticPauseChanged: livePropertyTimer.restart()

    Connections {
        target: overlayController
        function onActiveChanged() { if(!root.shuttingDown && !overlayController.active && root.actorUrls.length) {root.actorUrls=[];root.saveState()} }
    }

    function showStatus(message) {
        statusMessage = message
        statusTimer.restart()
    }

    function removeSelectedActor() {
        const next = actorUrls.filter((url) => url !== selectedUrl)
        if (!overlayController.applyActors(actorsConfiguration(next), selectedMonitor)) {
            showStatus(overlayController.lastError)
            return
        }
        actorUrls = next
        saveState()
        showStatus("Character removed from the desktop.")
    }

    Component.onCompleted: {
        restoreState()
        if (!applicationController.setStartWithSession(!!settings.startWithSession))
            showStatus("Could not update the autostart entry.")
        reconcileSelectedMonitor()
        restoreRequested = !!(resumeWallpaper || settings.restoreOnLaunch)
        if (restoreRequested) Qt.callLater(restoreSavedWallpapers)
    }
    onVisibleChanged: Qt.callLater(preparePreview)
    onSelectedUrlChanged: {pageFlick.contentY = 0; animationPreviewPaused=false; Qt.callLater(preparePreview)}
    onSelectedTabChanged: Qt.callLater(preparePreview)
    onRenderPixelWidthChanged: Qt.callLater(preparePreview)
    onRenderPixelHeightChanged: Qt.callLater(preparePreview)
    onMonitorNamesChanged: {
        restoreAttemptsByMonitor=({})
        restoreGiveUp=({})
        reconcileSelectedMonitor()
        Qt.callLater(restoreSavedWallpapers)
    }
    onAssetsChanged: {
        restoreGiveUp=({})
        Qt.callLater(restoreSavedWallpapers)
    }
    onSettingsChanged: {
        livePropertyTimer.restart()
        updateWindowPolicy()
        Qt.callLater(preparePreview)
    }
    onClosing: function(close) {
        close.accepted = false
        if(!settings.closeToTray) shuttingDown=true
        applicationController.closeWindow(!!settings.closeToTray)
    }

    Timer {
        id: wallpaperRestoreRetry
        interval: 1000
        repeat: true
        running: root.hasPendingConnectedWallpapers()
        onTriggered: root.restoreSavedWallpapers()
    }

    Timer {
        id: statusTimer
        interval: 7000
        onTriggered: root.statusMessage = ""
    }

    QtObject {
        id: wallpaperFiles
        function open() { const files = nativeFilePicker.openFiles("Import wallpapers", ["Images and videos (*.png *.jpg *.jpeg *.webp *.gif *.apng *.bmp *.svg *.mp4 *.mkv *.webm *.mov *.avi *.m4v)", "All files (*)"]); if (files.length) { root.importFiles(files, "wallpaper") } }
    }

    QtObject {
        id: animationFiles
        function open() { const files = nativeFilePicker.openFiles("Add animations", ["Animations and images (*.gif *.apng *.webp *.png *.jpg *.jpeg *.bmp)", "All files (*)"]); if (files.length) { root.importFiles(files, "character") } }
    }

    QtObject {
        id: genericFiles
        function open() { const files = nativeFilePicker.openFiles("Add content to library", ["Supported media (*.gif *.apng *.webp *.png *.jpg *.jpeg *.bmp *.svg *.mp4 *.mkv *.webm *.mov *.avi *.m4v)", "All files (*)"]); if (files.length) { importReview.pendingFiles = files.map(file => String(file)); importReview.open() } }
    }

    function preparePreview() {
        ++previewRequest
        previewUrl=""
        preparingPreview=false
        previewPlayer.stop()
        previewFrameLimiter.releaseFrame()
        if(!visible || selectedTab!=="installed" || !selectedAsset || !selectedAsset.video || !settings.animatePreview)return
        preparingPreview=true
        mediaOptimizer.preparePreviewAsync(selectedAsset.url,Number(selectedAsset.width || 0),Number(selectedAsset.height || 0),renderPixelWidth,renderPixelHeight,previewRequest)
    }
    function togglePreview() {
        if(!selectedAsset || !settings.animatePreview || preparingPreview)return
        if(selectedAsset.video) {
            if(previewPlayer.playbackState===MediaPlayer.PlayingState)previewPlayer.pause()
            else if(previewUrl.length)previewPlayer.play()
        } else if(selectedAsset.animated)animationPreviewPaused=!animationPreviewPaused
    }
    Engine.VideoFrameLimiter { id: previewFrameLimiter; objectName:"previewFrameLimiter"; fps: root.effectiveFps; target: previewOutput.videoSink }
    MediaPlayer {
        id: previewPlayer
        source: root.previewUrl
        videoOutput: previewFrameLimiter
        loops: root.selectedAsset && root.selectedAsset.loop === false ? 1 : MediaPlayer.Infinite
        playbackRate: root.selectedAsset ? (root.selectedAsset.speed ?? 100)/100 : 1
        audioOutput: AudioOutput { muted: true; device: root.selectedAudioDevice(root.settings.audioOutputId); volume: (root.selectedAsset ? root.selectedAsset.volume ?? 50 : 50) / 100 * (root.settings.globalVolume ?? 100) / 100 }
        onSourceChanged: {
            if (root.visible && source.toString().length > 0 && root.settings.animatePreview)
                play()
            else
                { stop(); previewFrameLimiter.releaseFrame() }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            id: navigationBar
            Layout.fillWidth: true
            Layout.preferredHeight: 64
            color: "#14121B"
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 24
                anchors.rightMargin: 24
                spacing: 16
                Image {source:"qrc:/icons/waniengine.png";Layout.preferredWidth:32;Layout.preferredHeight:32;sourceSize:Qt.size(64,64);fillMode:Image.PreserveAspectFit}
                Text { text: "WaniEngine"; color: "#F4F2FA"; font.pixelSize: 17; font.weight: Font.DemiBold; Layout.rightMargin: 28 }
                Repeater {
                    model: [{label:"Library",key:"installed"},{label:"Discover",key:"discover"},{label:"Animations",key:"animations"}]
                    delegate: Button_redesign {
                        required property var modelData
                        Layout.preferredWidth: 116
                        Layout.fillHeight: true
                        onClicked: {
                            root.selectedTab=modelData.key
                            root.contentFilter="all"
                            root.searchText=""
                            if(modelData.key !== "discover") {
                                const kind=modelData.key === "animations" ? "character" : "wallpaper"
                                if(!root.selectedAsset || root.selectedAsset.kind !== kind) {
                                    const first=root.assets.find(asset=>asset.kind === kind && !asset.missing)
                                    root.selectedUrl=first ? first.url : ""
                                }
                            }
                        }
                        background: Item {
                            Rectangle { anchors.bottom: parent.bottom; anchors.horizontalCenter: parent.horizontalCenter; width: parent.width-24; height: 2; radius: 1; color: "#928BD6"; visible: root.selectedTab === modelData.key }
                        }
                        contentItem: Text { text: modelData.label; color: root.selectedTab === modelData.key ? "#C5BFF0" : parent.hovered ? "#F4F2FA" : "#A9A4B9"; font.pixelSize: 14; font.weight: root.selectedTab === modelData.key ? Font.DemiBold : Font.Normal; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }
                }
                Item { Layout.fillWidth: true }
                Button_redesign { objectName: "settingsButton"; text: "Settings"; Layout.preferredWidth: 112; onClicked: settingsDialog.open() }
            }
        }

        Flickable {
            id: pageFlick
            objectName: "redesignPage"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentWidth: width
            contentHeight: pageContent.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

            ColumnLayout {
                id: pageContent
                width: pageFlick.width
                spacing: 12

                Item {
                    id: wallpaperHero
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.min(416, Math.max(224, pageFlick.height * 0.4))
                    clip: true
                    visible: root.selectedTab === "installed"

                    Rectangle { anchors.fill: parent; color: "#16141F" }

                    Image {
                        anchors.fill: parent
                        source: root.selectedAsset && root.selectedAsset.kind === "wallpaper"
                                ? (root.selectedAsset.thumbnailUrl || (root.selectedAsset.video ? "" : root.selectedAsset.url))
                                : root.selectedAsset && root.selectedAsset.kind === "character"
                                  ? root.selectedAsset.thumbnailUrl || root.selectedAsset.url : ""
                        fillMode: !root.selectedAsset || root.selectedAsset.fit === "cover" ? Image.PreserveAspectCrop : root.selectedAsset.fit === "stretch" ? Image.Stretch : root.selectedAsset.fit === "center" ? Image.Pad : Image.PreserveAspectFit
                        smooth: !root.selectedAsset || root.selectedAsset.smoothing !== false
                        asynchronous: true
                        sourceSize: Qt.size(root.renderPixelWidth,root.renderPixelHeight)
                        visible: source.toString().length > 0
                    }

                    VideoOutput {
                        id: previewOutput
                        anchors.fill: parent
                        fillMode: root.selectedAsset && root.selectedAsset.fit === "stretch" ? VideoOutput.Stretch : root.selectedAsset && root.selectedAsset.fit !== "cover" ? VideoOutput.PreserveAspectFit : VideoOutput.PreserveAspectCrop
                        visible: root.selectedAsset && root.selectedAsset.kind === "wallpaper" && root.selectedAsset.video
                    }

                    Engine.CappedAnimation {
                        id: inspectorAnimation
                        objectName: "animationPreview"
                        anchors.fill: parent
                        source: root.visible && root.selectedTab==="installed" && root.selectedAsset && root.selectedAsset.animated && !root.selectedAsset.video
                                ? root.selectedAsset.url : ""
                        fillMode: Image.PreserveAspectCrop
                        maximumSourceSize: Qt.size(root.renderPixelWidth,root.renderPixelHeight)
                        paused: !root.visible || !root.settings.animatePreview || root.animationPreviewPaused
                        repeat: !root.selectedAsset || root.selectedAsset.loop!==false
                        fps: root.effectiveFps
                        speed: root.selectedAsset ? Number(root.selectedAsset.speed || 100) / 100 : 1
                        visible: source.toString().length > 0
                    }

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        height: 112
                        gradient: Gradient {
                            GradientStop { position: 0; color: "#00000000" }
                            GradientStop { position: 1; color: "#CC100F16" }
                        }
                    }

                    ColumnLayout {
                        anchors.left: parent.left
                        anchors.bottom: parent.bottom
                        anchors.leftMargin: 28
                        anchors.bottomMargin: 20
                        spacing: 4
                        Text {
                            text: root.selectedAsset ? root.selectedAsset.name : root.assets.length === 0 ? "Your library starts here" : "Select a wallpaper"
                            color: "#F4F2FA"
                            font.pixelSize: 26
                            font.weight: Font.DemiBold
                            elide: Text.ElideRight
                        }
                        Text {
                            text: root.selectedAsset
                                  ? (root.selectedAsset.width ? root.selectedAsset.width + " × " + root.selectedAsset.height : root.selectedAsset.extension)
                                    + (root.selectedAsset.video ? "  ·  Video" : "  ·  Image")
                                    + (root.selectedAsset.nativeFps ? "  ·  " + Math.round(root.selectedAsset.nativeFps) + " FPS" : "")
                                  : root.assets.length === 0 ? "Import a wallpaper or character to personalize your desktop." : "Choose an item from your collection."
                            color: "#C0BDCF"
                            font.pixelSize: 13
                            elide: Text.ElideRight
                        }
                    }

                    Row {
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        anchors.rightMargin: 24
                        anchors.bottomMargin: 20
                        spacing: 12
                        Button_redesign {
                            width: 48; height: 48
                            enabled: root.selectedAsset && (root.selectedAsset.video || root.selectedAsset.animated) && root.settings.animatePreview && !root.preparingPreview
                            text: root.selectedAsset && root.selectedAsset.video ? (previewPlayer.playbackState === MediaPlayer.PlayingState ? "Ⅱ" : "▶") : (root.selectedAsset && root.selectedAsset.animated && !inspectorAnimation.paused ? "Ⅱ" : "▶")
                            onClicked: root.togglePreview()
                            background: Rectangle { radius: 24; color: "#D916141F"; border.color: "#383341" }
                            contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 17; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                        }
                        Button_redesign {
                            width: 48; height: 48
                            enabled: root.selectedAsset && root.selectedAsset.video
                            text: previewPlayer.audioOutput.muted ? "♪" : "♫"
                            onClicked: previewPlayer.audioOutput.muted = !previewPlayer.audioOutput.muted
                            background: Rectangle { radius: 24; color: "#D916141F"; border.color: "#383341" }
                            contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 17; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                        }
                    }

                    Text {
                        anchors.centerIn: parent
                        visible: root.selectedAsset === null && root.assets.length === 0
                        text: "▧"
                        color: "#9993AB"
                        font.pixelSize: 46
                    }
                }

                Rectangle {
                    id: wallpaperControls
                    Layout.fillWidth: true
                    Layout.preferredHeight: 72
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    radius: 12
                    color: "#1A1822"
                    visible: root.selectedTab === "installed"
                    readonly property bool wallpaperSelected: root.selectedAsset !== null && root.selectedAsset.kind === "wallpaper" && !root.selectedAsset.missing
                    readonly property bool animated: wallpaperSelected && (root.selectedAsset.video || root.selectedAsset.animated)
                    readonly property bool audioAvailable: wallpaperSelected && root.selectedAsset.video && root.selectedAsset.hasAudio !== false
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 14
                        spacing: 12
                        RowLayout {
                            Layout.preferredWidth: Math.min(240,root.width*0.18)
                            Layout.maximumWidth: 240
                            spacing: 8
                            Text { text:"Monitor"; color:"#C0BDCF"; font.pixelSize:12 }
                            ComboBox_redesign { objectName:"wallpaperMonitor"; Layout.fillWidth:true; model:root.monitorLabels; currentIndex:Math.max(0,root.monitorNames.indexOf(root.selectedMonitor)); onActivated:root.selectMonitor(root.monitorNames[index]) }
                        }
                        RowLayout {
                            Layout.preferredWidth: 162
                            Layout.maximumWidth: 162
                            spacing: 8
                            Text { text:"Fit"; color:"#C0BDCF"; font.pixelSize:12 }
                            ComboBox_redesign {
                                objectName:"wallpaperFit"; Layout.fillWidth:true; enabled:wallpaperControls.wallpaperSelected
                                model:["Cover","Contain","Stretch","Center"]
                                currentIndex:root.selectedAsset ? Math.max(0,["cover","contain","stretch","center"].indexOf(root.selectedAsset.fit || "cover")) : 0
                                onActivated:root.updateAsset(root.selectedUrl,"fit",["cover","contain","stretch","center"][index])
                            }
                        }
                        Rectangle { width:1; Layout.preferredHeight:28; color:"#34303F" }
                        RowLayout {
                            Layout.fillWidth:true
                            Layout.minimumWidth: 216
                            spacing: 10
                            Toggle_redesign { objectName:"wallpaperAudio"; text:"Audio"; Layout.preferredWidth:96; enabled:wallpaperControls.audioAvailable; checked:root.selectedAsset ? !!root.selectedAsset.audioEnabled : false; onToggled:root.updateAsset(root.selectedUrl,"audioEnabled",checked) }
                            Slider { objectName:"wallpaperVolume"; Layout.fillWidth:true; Layout.minimumWidth:64; enabled:wallpaperControls.audioAvailable; from:0;to:100;value:root.selectedAsset ? root.selectedAsset.volume ?? 50 : 50;onMoved:root.updateAsset(root.selectedUrl,"volume",Math.round(value));Accessible.name:"Wallpaper volume" }
                            Text { text:(root.selectedAsset ? root.selectedAsset.volume ?? 50 : 50)+"%"; color:wallpaperControls.audioAvailable ? "#C0BDCF" : "#777185"; font.pixelSize:12; Layout.preferredWidth:32 }
                        }
                        Rectangle { width:1; Layout.preferredHeight:28; color:"#34303F" }
                        Toggle_redesign { objectName:"wallpaperLoop"; text:"Loop"; Layout.preferredWidth:88; enabled:wallpaperControls.animated; checked:root.selectedAsset ? root.selectedAsset.loop !== false : true; onToggled:root.updateAsset(root.selectedUrl,"loop",checked) }
                        Button_redesign { objectName:"wallpaperAdjustments"; text:"Adjustments"; Layout.preferredWidth:112; enabled:wallpaperControls.wallpaperSelected; checkable:true;checked:root.showWallpaperAdjustments;onClicked:root.showWallpaperAdjustments=!root.showWallpaperAdjustments }
                        Button_redesign { objectName:"applyButton"; text:root.busyApplying ? "Preparing…" : "Apply wallpaper"; primary:true; Layout.preferredWidth:168; enabled:wallpaperControls.wallpaperSelected && !root.busyApplying;onClicked:root.applySelected() }
                    }
                }

                RowLayout {
                    id: wallpaperAdvanced
                    Layout.fillWidth: true
                    Layout.preferredHeight: 184
                    Layout.leftMargin: 12
                    Layout.rightMargin: 12
                    spacing: 12
                    visible: root.selectedTab === "installed" && root.showWallpaperAdjustments && root.selectedAsset && root.selectedAsset.kind === "wallpaper"

                    Repeater {
                        model: [
                            { title: "Framing", keys: ["zoom", "offsetX", "offsetY"], labels: ["Zoom", "Offset X", "Offset Y"], mins: [100, -100, -100], maxs: [200, 100, 100], defaults: [100, 0, 0], suffixes: ["%", "%", "%"] },
                            { title: "Playback", keys: ["speed", "volume"], labels: ["Speed", "Volume"], mins: [25, 0], maxs: [200, 100], defaults: [100, 50], suffixes: ["%", "%"] },
                            { title: "Appearance", keys: [], labels: [], mins: [], maxs: [], defaults: [], suffixes: [] }
                        ]
                        delegate: Rectangle {
                            required property var modelData
                            readonly property var group: modelData
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            radius: 12
                            color: "#1A1822"
                            border.color: "#383341"
                            ColumnLayout {
                                anchors.fill: parent
                                anchors.margins: 12
                                spacing: 4
                                Text { text: group.title; color: "#F4F2FA"; font.pixelSize: 14; font.weight: Font.DemiBold }
                                Repeater {
                                    model: group.keys.length
                                    delegate: PropertySlider_redesign {
                                        required property int index
                                        Layout.fillWidth: true
                                        Layout.preferredHeight: 36
                                        label: group.labels[index]
                                        from: group.mins[index]
                                        to: group.maxs[index]
                                        defaultValue: group.defaults[index]
                                        suffix: group.suffixes[index]
                                        value: root.selectedAsset ? root.selectedAsset[group.keys[index]] ?? group.defaults[index] : group.defaults[index]
                                        visible: group.keys[index] !== "volume" || (root.selectedAsset && root.selectedAsset.hasAudio !== false)
                                        onEdited: value => root.updateAsset(root.selectedUrl, group.keys[index], value)
                                    }
                                }
                                RowLayout {
                                    visible: group.title !== "Framing"
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 40
                                    spacing: 16
                                    Toggle_redesign {
                                        visible: group.title === "Playback"
                                        text: "Play"
                                        checked: root.selectedAsset ? root.selectedAsset.playing !== false : true
                                        onToggled: root.updateAsset(root.selectedUrl, "playing", checked)
                                        palette.text: "#C0BDCF"
                                    }
                                    Toggle_redesign {
                                        visible: group.title === "Playback"
                                        text: "Loop"
                                        checked: root.selectedAsset ? root.selectedAsset.loop !== false : true
                                        onToggled: root.updateAsset(root.selectedUrl, "loop", checked)
                                        palette.text: "#C0BDCF"
                                    }
                                    Toggle_redesign {
                                        visible: group.title === "Appearance"
                                        text: "Mirror"
                                        checked: root.selectedAsset ? !!root.selectedAsset.flipX : false
                                        onToggled: root.updateAsset(root.selectedUrl, "flipX", checked)
                                        palette.text: "#C0BDCF"
                                    }
                                    Toggle_redesign {
                                        visible: group.title === "Appearance"
                                        text: "Smooth scaling"
                                        checked: root.selectedAsset ? root.selectedAsset.smoothing !== false : true
                                        onToggled: root.updateAsset(root.selectedUrl, "smoothing", checked)
                                        palette.text: "#C0BDCF"
                                    }
                                }
                            }
                        }
                    }
                }

                RowLayout {
                    id: libraryToolbar
                    Layout.fillWidth: true
                    Layout.preferredHeight: 56
                    Layout.leftMargin: 24
                    Layout.rightMargin: 24
                    spacing: 8
                    visible: root.selectedTab === "installed"

                    Text {
                        Layout.preferredWidth: 170
                        text: "Library  ·  " + root.visibleAssets.length
                        color: "#F4F2FA"
                        font.pixelSize: 19
                        font.weight: Font.DemiBold
                        elide: Text.ElideRight
                    }

                    TextField {
                        id: searchField
                        Layout.fillWidth: true
                        Layout.minimumWidth: 150
                        Layout.preferredHeight: 40
                        placeholderText: "Search your library…"
                        text: root.searchText
                        onTextEdited: root.searchText = text
                        leftPadding: 40
                        rightPadding: 40
                        color: "#F4F2FA"
                        placeholderTextColor: "#9993AB"
                        font.pixelSize: 13
                        selectByMouse: true
                        background: Rectangle { color: "#16141F"; border.width: 1; border.color: searchField.activeFocus ? "#7872C9" : "#383341"; radius: 10 }
                        Text { anchors.left: parent.left; anchors.leftMargin: 12; anchors.verticalCenter: parent.verticalCenter; text: "⌕"; color: "#9993AB"; font.pixelSize: 22 }
                        Button_redesign {
                            visible: searchField.text.length > 0
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            width: 32; height: 32
                            text: "×"
                            background: Rectangle { color: "transparent"; radius: 16 }
                            contentItem: Text { text: parent.text; color: "#C0BDCF"; font.pixelSize: 18; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                            onClicked: { root.searchText = ""; searchField.text = "" }
                        }
                    }

                    ComboBox_redesign {
                        id: kindFilter
                        Layout.preferredWidth: 128
                        Layout.preferredHeight: 40
                        model: ["All types", "Wallpapers", "Animations"]
                        currentIndex: root.contentFilter === "wallpaper" ? 1 : root.contentFilter === "character" ? 2 : 0
                        onActivated: root.contentFilter = index === 1 ? "wallpaper" : index === 2 ? "character" : "all"
                        contentItem: Text { leftPadding: 12; rightPadding: 24; text: kindFilter.displayText; color: "#C0BDCF"; font.pixelSize: 12; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
                        background: Rectangle { radius: 10; color: "#16141F"; border.color: "#383341" }
                    }

                    Button_redesign {
                        text: "Favorites"
                        Layout.preferredWidth: 112
                        Layout.preferredHeight: 40
                        onClicked: root.favoritesOnly = !root.favoritesOnly
                        background: Rectangle { radius: 10; color: root.favoritesOnly ? "#302B43" : "#16141F"; border.color: root.favoritesOnly ? "#7872C9" : "#383341" }
                        contentItem: Text { text: parent.text; color: root.favoritesOnly ? "#7872C9" : "#C0BDCF"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }

                    Button_redesign {
                        text: "Import"
                        Layout.preferredWidth: 136
                        Layout.preferredHeight: 40
                        onClicked: genericFiles.open()
                        background: Rectangle { radius: 10; color: "#242130"; border.color: "#383341" }
                        contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }

                    Button_redesign {
                        text: "Reload"
                        Layout.preferredWidth: 104
                        Layout.preferredHeight: 40
                        onClicked: root.reloadLibrary()
                        background: Rectangle { radius: 10; color: "#242130"; border.color: "#383341" }
                        contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                    }

                    ComboBox_redesign {
                        id: sortBox
                        Layout.preferredWidth: 164
                        Layout.preferredHeight: 40
                        model: ["Most recent", "Name", "Type"]
                        currentIndex: root.sortMode === "name" ? 1 : root.sortMode === "type" ? 2 : 0
                        onActivated: root.sortMode = index === 1 ? "name" : index === 2 ? "type" : "recent"
                        contentItem: Text { leftPadding: 12; rightPadding: 24; text: sortBox.displayText; color: "#C0BDCF"; font.pixelSize: 12; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
                        background: Rectangle { radius: 10; color: "#16141F"; border.color: "#383341" }
                    }
                }

                Item {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.max(190, pageFlick.height-wallpaperHero.height-wallpaperControls.height-56-36)
                    Layout.leftMargin: 24
                    Layout.rightMargin: 24
                    visible: root.selectedTab === "installed"

                    GridView {
                        id: gallery
                        objectName: "libraryGallery"
                        anchors.fill: parent
                        clip: true
                        boundsBehavior: Flickable.StopAtBounds
                        cellWidth: width / Math.max(1, Math.floor((width + 16) / 236))
                        cellHeight: Math.round((cellWidth - 16) * 9 / 16) + 60
                        model: root.visibleAssets
                        visible: count > 0
                        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

                        delegate: Item {
                            id: cardRoot
                            required property var modelData
                            width: gallery.cellWidth - 16
                            height: gallery.cellHeight - 12

                            Rectangle {
                                anchors.fill: parent
                                radius: 10
                                clip: true
                                color: "#1A1822"
                                border.width: root.selectedUrl === modelData.url ? 2 : hoveredArea.containsMouse ? 1 : 0
                                border.color: root.selectedUrl === modelData.url ? "#7872C9" : "#383341"

                                Image {
                                    sourceSize: Qt.size(Math.max(1,Math.ceil(width*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(height*(root.Screen.devicePixelRatio || 1))))
                                    id: thumbnail
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    height: Math.round((parent.width - 4) * 9 / 16)
                                    anchors.margins: 2
                                    source: modelData.thumbnailUrl || (modelData.video ? "" : modelData.url)
                                    fillMode: modelData.kind === "character" ? Image.PreserveAspectFit : Image.PreserveAspectCrop
                                    asynchronous: true
                                    cache: true
                                    visible: status !== Image.Error && status !== Image.Null
                                }

                                Text {
                                    anchors.centerIn: thumbnail
                                    visible: modelData.video || thumbnail.status === Image.Error || thumbnail.status === Image.Null
                                    text: modelData.video ? "▶" : modelData.extension
                                    color: "#C0BDCF"
                                    font.pixelSize: modelData.video ? 24 : 16
                                }

                                Rectangle {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    height: 48
                                    color: "transparent"
                                    Column {
                                        anchors.fill: parent
                                        anchors.leftMargin: 10
                                        anchors.rightMargin: 10
                                        anchors.topMargin: 4
                                        spacing: 2
                                        Text { width: parent.width; text: modelData.name; color: "#F4F2FA"; font.pixelSize: 13; font.weight: Font.DemiBold; elide: Text.ElideRight }
                                        Text { width: parent.width; text: (modelData.kind === "character" ? "Personagem" : modelData.video ? "Video" : "Image") + (modelData.width ? "  ·  " + modelData.width + " × " + modelData.height : ""); color: "#9993AB"; font.pixelSize: 11; elide: Text.ElideRight }
                                    }
                                }

                                Rectangle {
                                    visible: plasmaIntegration.active && modelData.url === root.backgroundUrl
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    anchors.margins: 8
                                    width: 24; height: 24; radius: 12
                                    color: "#D916141F"
                                    Text { anchors.centerIn: parent; text: "✓"; color: "#7872C9"; font.pixelSize: 14; font.bold: true }
                                }

                                MouseArea {
                                    id: hoveredArea
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                                    onClicked: {
                                        root.selectedUrl = modelData.url
                                        if (mouse.button === Qt.RightButton)
                                            cardMenu.popup()
                                        else if (modelData.kind === "character")
                                            root.navigate("animations")
                                    }
                                    onDoubleClicked: root.applySelected()
                                }

                                Button_redesign {
                                    z: 3
                                    anchors.left: parent.left
                                    anchors.top: parent.top
                                    anchors.margins: 8
                                    width: 36; height: 36
                                    text: modelData.favorite ? "♥" : "♡"
                                    background: Rectangle { color: "#D916141F"; radius: 18 }
                                    contentItem: Text { text: parent.text; color: modelData.favorite ? "#FB7185" : "#F4F2FA"; font.pixelSize: 18; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                                    onClicked: root.toggleFavorite(modelData.url)
                                }
                            }
                            ToolTip.visible: hoveredArea.containsMouse
                            ToolTip.text: modelData.name + "\n" + modelData.extension + " · " + (modelData.kind === "wallpaper" ? "Wallpaper" : "Animation")
                            ToolTip.delay: 500
                        }
                    }

                    ColumnLayout {
                        anchors.centerIn: parent
                        visible: root.visibleAssets.length === 0
                        spacing: 12
                        Text {
                            Layout.alignment: Qt.AlignHCenter
                            text: root.assets.length === 0 ? "Your library starts here" : "No results found"
                            color: "#F4F2FA"
                            font.pixelSize: 20
                            font.weight: Font.DemiBold
                        }
                        Text {
                            Layout.alignment: Qt.AlignHCenter
                            text: root.assets.length === 0 ? "Import a wallpaper or character." : "Try a different search or clear your filters."
                            color: "#9993AB"
                            font.pixelSize: 13
                        }
                        Button_redesign {
                            visible: root.assets.length === 0
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredHeight: 44
                            Layout.preferredWidth: 200
                            text: "Import content"
                            onClicked: genericFiles.open()
                            background: Rectangle { radius: 10; color: "#7872C9" }
                            contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 13; font.weight: Font.DemiBold; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                        }
                        Button_redesign {
                            visible: root.assets.length > 0
                            Layout.alignment: Qt.AlignHCenter
                            text: "Clear search and filters"
                            onClicked: { root.searchText = ""; searchField.text = ""; root.contentFilter = "all"; root.favoritesOnly = false }
                            background: Rectangle { radius: 10; color: "#242130"; border.color: "#383341" }
                            contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 13; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                        }
                    }
                }

                AnimaLibrary_redesign {
                    id: animationPage
                    objectName: "animationPage"
                    Layout.fillWidth: true
                    Layout.leftMargin: 24
                    Layout.rightMargin: 24
                    Layout.topMargin: 16
                    Layout.bottomMargin: 16
                    visible: root.selectedTab === "animations"
                    assets: root.visibleAssets
                    selectedAsset: root.selectedAsset && root.selectedAsset.kind === "character" ? root.selectedAsset : null
                    activeUrls: overlayController.active ? root.actorUrls : []
                    selectedMonitor: root.selectedMonitor
                    monitorNames: root.monitorNames
                    monitorLabels: root.monitorLabels
                    backgroundUrl: root.backgroundUrl
                    backgroundAsset: root.assets.find(asset=>asset.url===root.backgroundUrl) || null
                    fpsLimit: root.effectiveFps
                    searchText: root.searchText
                    screenWidth: root.selectedScreen.width
                    screenHeight: root.selectedScreen.height
                    animate: root.visible && root.settings.animatePreview
                    onSelected: (url) => root.selectedUrl = url
                    onMonitorSelected: (name) => root.selectMonitor(name)
                    onImportRequested: animationFiles.open()
                    onAddAudioRequested: actorAudioFiles.open()
                    onSoundSettingsRequested: (index) => { soundSettings.trackIndex = index; soundSettings.open() }
                    onUpdateRequested: (url, key, value) => root.updateAsset(url, key, value)
                    onAudioEnabledRequested: (enabled) => root.updateAudio("enabled", enabled)
                    onApplyRequested: root.applySelected()
                    onRemoveRequested: root.removeSelectedActor()
                    onDeleteRequested: removeDialog.open()
                    onSearchEdited: (text) => root.searchText = text
                }

                Item {
                    id: discoveryPage
                    visible: root.selectedTab === "discover"
                    Layout.fillWidth: true
                    Layout.preferredHeight: discoveryItems.length ? discoveryContent.implicitHeight+48 : Math.max(420,pageFlick.height-24)
                    property string filter: "all"
                    readonly property var discoveryItems: root.assets.filter(asset=>asset.kind==="wallpaper" && !asset.missing && (filter==="all" || (filter==="video" ? asset.video : !asset.video)))
                    ColumnLayout {
                        id: discoveryContent
                        visible: discoveryPage.discoveryItems.length>0
                        anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top; anchors.margins:24
                        spacing: 20
                        RowLayout {
                            Layout.fillWidth:true
                            ColumnLayout { spacing:4; Text {text:"Discover";color:"#F4F2FA";font.pixelSize:26;font.weight:Font.DemiBold} Text {text:"Explore wallpapers from your collection";color:"#9993AB";font.pixelSize:13} }
                            Item {Layout.fillWidth:true}
                            ComboBox_redesign {model:["All wallpapers","Videos","Images"];Layout.preferredWidth:180;onActivated:discoveryPage.filter=["all","video","image"][index]}
                        }
                        GridLayout {
                            Layout.fillWidth:true
                            columns:Math.max(3,Math.floor((root.width-48)/240))
                            columnSpacing:16;rowSpacing:20
                            Repeater {
                                model:discoveryPage.discoveryItems
                                delegate: Rectangle {
                                    required property var modelData
                                    Layout.fillWidth:true;Layout.preferredHeight:190;Layout.minimumWidth:0
                                    radius:12;color:"#1A1822";clip:true
                                    Image {anchors.left:parent.left;anchors.right:parent.right;anchors.top:parent.top;height:144;source:modelData.thumbnailUrl || (modelData.video ? "" : modelData.url);fillMode:Image.PreserveAspectCrop;asynchronous:true; sourceSize: Qt.size(Math.max(1,Math.ceil(width*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(height*(root.Screen.devicePixelRatio || 1)))) }
                                    Text {anchors.left:parent.left;anchors.right:parent.right;anchors.bottom:parent.bottom;anchors.margins:12;text:modelData.name;color:"#F4F2FA";font.pixelSize:13;elide:Text.ElideRight}
                                    MouseArea {anchors.fill:parent;onClicked:{root.selectedTab="installed";root.selectedUrl=modelData.url}}
                                }
                            }
                        }
                    }
                    ColumnLayout {
                        visible:discoveryPage.discoveryItems.length===0
                        anchors.centerIn:parent
                        width:Math.min(520,parent.width-64)
                        spacing:16
                        Text {Layout.alignment:Qt.AlignHCenter;text:"Explore your next wallpaper";color:"#F4F2FA";font.pixelSize:26;font.weight:Font.DemiBold;horizontalAlignment:Text.AlignHCenter;Layout.fillWidth:true;wrapMode:Text.WordWrap}
                        Text {Layout.fillWidth:true;text:"Import a few wallpapers to start exploring your collection.";color:"#9993AB";font.pixelSize:14;horizontalAlignment:Text.AlignHCenter;wrapMode:Text.WordWrap}
                        RowLayout {Layout.alignment:Qt.AlignHCenter;Layout.topMargin:12;spacing:12;Button_redesign {text:"Import wallpapers";primary:true;onClicked:wallpaperFiles.open()} Button_redesign {text:"Open library";onClicked:root.selectedTab="installed"} }
                    }
                }

            }
        }

        Rectangle {
            id: playlistDock
            Layout.fillWidth: true
            Layout.preferredHeight: 88
            color: "#14121B"
            border.color: "#2C2836"

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 16
                anchors.rightMargin: 16
                spacing: 8

                Text {
                    Layout.preferredWidth: 160
                    text: "♫  " + root.playlistName + (root.playlistDirty ? "  •" : "") + "  ·  " + root.playlist.length
                    color: "#F4F2FA"
                    font.pixelSize: 13
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }

                Button_redesign {
                    text: "Load"
                    Layout.preferredWidth: 96; Layout.preferredHeight: 40
                    onClicked: playlistLoadFiles.open()
                    background: Rectangle { radius: 9; color: "#16141F"; border.color: "#383341" }
                    contentItem: Text { text: parent.text; color: "#C0BDCF"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }
                Button_redesign {
                    text: "Save as"
                    Layout.preferredWidth: 112; Layout.preferredHeight: 40
                    enabled: root.playlist.length > 0
                    onClicked: playlistSaveFiles.open()
                    background: Rectangle { radius: 9; color: "#16141F"; border.color: "#383341" }
                    contentItem: Text { text: parent.text; color: parent.enabled ? "#C0BDCF" : "#777185"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }
                Button_redesign {
                    text: "Manage"
                    Layout.preferredWidth: 104; Layout.preferredHeight: 40
                    onClicked: playlistDialog.open()
                    background: Rectangle { radius: 9; color: "#16141F"; border.color: "#383341" }
                    contentItem: Text { text: parent.text; color: "#C0BDCF"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }

                Flickable {
                    id: playlistView
                    Layout.fillWidth: true
                    Layout.preferredHeight: 64
                    contentWidth: Math.max(width, playlistRow.implicitWidth)
                    contentHeight: height
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    Row {
                        id: playlistRow
                        spacing: 8
                        Repeater {
                            model: root.playlist
                            delegate: Item {
                                id: playlistCard
                                required property string modelData
                                width: 72; height: 56
                                property var itemData: root.assets.find(entry => entry.url === modelData)
                                Rectangle {
                                    anchors.fill: parent
                                    radius: 8
                                    clip: true
                                    color: "#1A1822"
                                    border.width: root.selectedUrl === playlistCard.modelData ? 2 : 1
                                    border.color: root.selectedUrl === playlistCard.modelData ? "#7872C9" : "#383341"
                                    Image { anchors.fill: parent; source: playlistCard.itemData ? playlistCard.itemData.thumbnailUrl || (playlistCard.itemData.video ? "" : playlistCard.itemData.url) : ""; fillMode: Image.PreserveAspectCrop; asynchronous: true ; sourceSize: Qt.size(Math.max(1,Math.ceil(width*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(height*(root.Screen.devicePixelRatio || 1)))) }
                                    MouseArea { anchors.fill: parent; onClicked: root.selectedUrl = playlistCard.modelData }
                                }
                            }
                        }
                    }
                }

                Button_redesign {
                    text: "⇄"
                    Layout.preferredWidth: 40; Layout.preferredHeight: 40
                    enabled: root.playlist.length > 0
                    checkable: true
                    checked: !!root.settings.playlistRandom
                    onToggled: { root.settings = Object.assign({}, root.settings, {playlistRandom: checked}); root.saveState() }
                    background: Rectangle { radius: 20; color: parent.checked ? "#302B43" : "transparent" }
                    contentItem: Text { text: parent.text; color: parent.enabled ? "#C0BDCF" : "#777185"; font.pixelSize: 20; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }

                Button_redesign {
                    objectName: "playlistStartButton"
                    text: root.playlistRunning ? "Ⅱ" : "▶"
                    Layout.preferredWidth: 52; Layout.preferredHeight: 52
                    enabled: root.playlist.length > 0
                    onClicked: root.togglePlaylist()
                    background: Rectangle { radius: 26; color: parent.enabled ? "#7872C9" : "#242130" }
                    contentItem: Text { text: parent.text; color: parent.enabled ? "#F4F2FA" : "#777185"; font.pixelSize: 18; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }

                Button_redesign {
                    text: "＋"
                    Layout.preferredWidth: 40; Layout.preferredHeight: 40
                    enabled: root.selectedAsset !== null && root.selectedAsset.kind === "wallpaper" && root.playlist.indexOf(root.selectedUrl) < 0
                    onClicked: root.addSelectedToPlaylist()
                    background: Rectangle { radius: 20; color: "#242130"; border.color: "#383341" }
                    contentItem: Text { text: parent.text; color: parent.enabled ? "#7872C9" : "#777185"; font.pixelSize: 21; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
                }

                Text {
                    visible: root.statusMessage.length > 0
                    Layout.preferredWidth: 208
                    text: root.statusMessage
                    color: "#C0BDCF"
                    font.pixelSize: 11
                    elide: Text.ElideRight
                    ToolTip.visible: statusHover.containsMouse
                    ToolTip.text: root.statusMessage
                    MouseArea {id:statusHover;anchors.fill:parent;hoverEnabled:true;acceptedButtons:Qt.NoButton}
                }
            }
        }
    }

    Popup {
        id: filterPopup
        parent: Overlay.overlay
        x: 24 + 170 + 8 + searchField.width + 8
        y: navigationBar.height + wallpaperHero.height + wallpaperControls.height + 40
        width: 280
        height: 284
        padding: 16
        modal: false
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        background: Rectangle { radius: 12; color: "#242130"; border.color: "#383341" }
        ColumnLayout {
            anchors.fill: parent
            spacing: 8
            Text { text: "Filters"; color: "#F4F2FA"; font.pixelSize: 16; font.weight: Font.DemiBold }
            Rectangle { Layout.fillWidth: true; height: 1; color: "#2C2836" }
            Text { text: "Content type"; color: "#9993AB"; font.pixelSize: 12 }
            RadioButton { text: "All"; checked: root.contentFilter === "all"; onClicked: root.contentFilter = "all"; palette.text: "#C0BDCF" }
            RadioButton { text: "Wallpapers"; checked: root.contentFilter === "wallpaper"; onClicked: root.contentFilter = "wallpaper"; palette.text: "#C0BDCF" }
            RadioButton { text: "Animations"; checked: root.contentFilter === "character"; onClicked: root.contentFilter = "character"; palette.text: "#C0BDCF" }
            Toggle_redesign { text: "Favorites only"; checked: root.favoritesOnly; onToggled: root.favoritesOnly = checked; palette.text: "#C0BDCF" }
            Button_redesign {
                Layout.fillWidth: true
                text: "Clear filters"
                onClicked: { root.contentFilter = "all"; root.favoritesOnly = false; root.searchText = ""; searchField.text = ""; filterPopup.close() }
                background: Rectangle { radius: 9; color: "#16141F"; border.color: "#383341" }
                contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
            }
        }
    }


    Menu {
        id: cardMenu
        MenuItem { text: "Apply"; onTriggered: root.applySelected() }
        MenuItem { text: "Add to playlist"; onTriggered: root.addSelectedToPlaylist() }
        MenuItem { text: root.selectedAsset && root.selectedAsset.favorite ? "Remove favorite" : "Add favorite"; onTriggered: if (root.selectedAsset) root.toggleFavorite(root.selectedUrl) }
        MenuItem { text:"Open containing folder";enabled:root.selectedAsset && !!root.selectedAsset.folderUrl;onTriggered:sceneStore.openFolder(root.selectedAsset.folderUrl) }
        MenuSeparator {}
        MenuItem { text: "Remove from library…"; onTriggered: removeDialog.open() }
    }

    Dialog_redesign {
        id: importReview
        modal: true
        anchors.centerIn: Overlay.overlay
        width: 640
        height: 440
        title: "Import content"
        standardButtons: Dialog.Ok | Dialog.Cancel
        property var pendingFiles: []
        property string importKind: "wallpaper"

        onAccepted: root.importFiles(pendingFiles, importKind)

        contentItem: RowLayout {
            spacing: 12
            Rectangle {
                Layout.preferredWidth: 220
                Layout.preferredHeight: 220
                color: root.field
                clip: true
                Image {
                    sourceSize: Qt.size(Math.max(1,Math.ceil(width*(root.Screen.devicePixelRatio || 1))),Math.max(1,Math.ceil(height*(root.Screen.devicePixelRatio || 1))))
                    anchors.fill: parent
                    source: importReview.pendingFiles.length > 0
                            && !root.isVideoFile(importReview.pendingFiles[0]) ? importReview.pendingFiles[0] : ""
                    fillMode: Image.PreserveAspectFit
                }
                Text {
                    anchors.centerIn: parent
                    visible: importReview.pendingFiles.length > 0 && root.isVideoFile(importReview.pendingFiles[0])
                    text: "▶"
                    color: root.textMuted
                    font.pixelSize: 24
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 8
                Text { text: "Selected files"; color: root.textMain; font.pixelSize: 12; font.bold: true }
                Text {
                    Layout.fillWidth: true
                    text: importReview.pendingFiles.slice(0, 4).map((url) => root.fileName(url)).join("\n")
                         + (importReview.pendingFiles.length > 4 ? "\n+" + (importReview.pendingFiles.length - 4) + " files" : "")
                    color: root.textMuted
                    font.pixelSize: 11
                    wrapMode: Text.WordWrap
                    maximumLineCount: 8
                }
                Item { Layout.preferredHeight: 4 }
                Text { text: "Import as"; color: root.textMuted; font.pixelSize: 11 }
                ComboBox_redesign {
                    Layout.fillWidth: true
                    model: ["Wallpaper", "Desktop character"]
                    currentIndex: importReview.importKind === "character" ? 1 : 0
                    onActivated: importReview.importKind = index === 0 ? "wallpaper" : "character"
                }
                Text {
                    Layout.fillWidth: true
                    text: "Choose how these files appear on your desktop."
                    color: root.textDisabled
                    font.pixelSize: 10
                    wrapMode: Text.WordWrap
                }
            }
        }
    }

    component SettingsTab: TabButton {
        background: Item { Rectangle { anchors.bottom:parent.bottom; width:parent.width; height:2; color:parent.parent.checked ? "#928BD6" : "transparent" } }
        contentItem: Text { text: parent.text; color: root.textMain; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
    }

    Dialog_redesign {
        id: settingsDialog
        objectName: "settingsDialog"
        width: Math.min(820,root.width-64)
        height: Math.min(660,root.height-64)
        title: "Settings"
        standardButtons: Dialog.Ok | Dialog.Cancel
        property string page: "performance"
        property var draft: ({})
        onOpened: draft=Object.assign({},root.settings)
        onAccepted: {
            root.settings=Object.assign({},draft,{fpsLimit:Math.max(15,Math.min(root.maxFps,draft.fpsLimit || 60)),samples:root.sampleLevels.includes(draft.samples) ? draft.samples : 0})
            if(!applicationController.setStartWithSession(!!root.settings.startWithSession)) root.showStatus("Could not update the autostart entry.")
            root.saveState()
            root.updateWindowPolicy()
            if(!plasmaIntegration.updateAudioOptions(root.audioConfiguration())) root.showStatus(plasmaIntegration.lastError)
            livePropertyTimer.restart()
        }
        contentItem: ColumnLayout {
            spacing:24
            TabBar {
                id:settingsTabs
                Layout.fillWidth:true
                Layout.preferredHeight:44
                spacing:12
                background:Rectangle {color:"transparent"}
                currentIndex:["performance","audio","general","library","about"].indexOf(settingsDialog.page)
                onCurrentIndexChanged:settingsDialog.page=["performance","audio","general","library","about"][currentIndex]
                SettingsTab {text:"Performance"}
                SettingsTab {text:"Audio"}
                SettingsTab {text:"General"}
                SettingsTab {text:"Library"}
                SettingsTab {text:"About"}
            }
            StackLayout {
                Layout.fillWidth:true;Layout.fillHeight:true
                currentIndex:settingsTabs.currentIndex
                ColumnLayout {
                    spacing:18
                    Text {text:"Playback behavior";color:root.textMain;font.pixelSize:16;font.weight:Font.DemiBold}
                    Toggle_redesign {Layout.fillWidth:true;text:"Pause when an application is fullscreen";checked:!!settingsDialog.draft.pauseFullscreen;onToggled:settingsDialog.draft=Object.assign({},settingsDialog.draft,{pauseFullscreen:checked})}
                    Toggle_redesign {Layout.fillWidth:true;text:"Pause when an application is maximized";checked:!!settingsDialog.draft.pauseMaximized;onToggled:settingsDialog.draft=Object.assign({},settingsDialog.draft,{pauseMaximized:checked})}
                    Rectangle {Layout.fillWidth:true;Layout.preferredHeight:1;color:root.divider}
                    RowLayout {Layout.fillWidth:true;spacing:16;Text {text:"Frame rate limit";color:root.textMuted;font.pixelSize:13} Item {Layout.fillWidth:true} SpinBox_redesign {objectName:"fpsLimitControl";from:15;to:root.maxFps;editable:true;value:settingsDialog.draft.fpsLimit || 60;onValueModified:settingsDialog.draft=Object.assign({},settingsDialog.draft,{fpsLimit:value})} }
                    Text {Layout.fillWidth:true;text:"Refresh rate: "+root.maxFps+" Hz. The limit caps presentation without generating extra video frames.";color:root.textDisabled;font.pixelSize:12;wrapMode:Text.WordWrap}
                    RowLayout {Layout.fillWidth:true;spacing:16;Text {text:"Edge antialiasing (MSAA)";color:root.textMuted;font.pixelSize:13} Item {Layout.fillWidth:true} ComboBox_redesign {objectName:"msaaControl";Layout.preferredWidth:180;model:root.sampleLevels.map(samples=>samples ? samples+"×" : "Off");currentIndex:Math.max(0,root.sampleLevels.indexOf(settingsDialog.draft.samples || 0));onActivated:settingsDialog.draft=Object.assign({},settingsDialog.draft,{samples:root.sampleLevels[index]})} }
                    Text {Layout.fillWidth:true;text:"Smooths transformed edges. Source images and video retain their original detail.";color:root.textDisabled;font.pixelSize:12;wrapMode:Text.WordWrap}
                    Item {Layout.fillHeight:true}
                }
                ColumnLayout {
                    spacing:18
                    Text {text:"Audio output";color:root.textMain;font.pixelSize:16;font.weight:Font.DemiBold}
                    ComboBox_redesign {
                        objectName:"audioOutputSelector";Layout.fillWidth:true
                        model:{let rows=[{label:"System default"+(audioDevices.defaultAudioOutput.description ? " — "+audioDevices.defaultAudioOutput.description : ""),deviceId:""}];for(let i=0;i<audioDevices.audioOutputs.length;++i)rows.push({label:audioDevices.audioOutputs[i].description,deviceId:String(audioDevices.audioOutputs[i].id)});return rows}
                        textRole:"label"
                        currentIndex:Math.max(0,model.findIndex(item=>item.deviceId===(settingsDialog.draft.audioOutputId || "")))
                        onActivated:settingsDialog.draft=Object.assign({},settingsDialog.draft,{audioOutputId:model[index].deviceId})
                    }
                    Text {Layout.fillWidth:true;text:"The system default follows Plasma's output selection. Disconnected devices fall back to the system default.";color:root.textDisabled;font.pixelSize:12;wrapMode:Text.WordWrap}
                    PropertySlider_redesign {objectName:"property:globalVolume";Layout.fillWidth:true;label:"Master volume";defaultValue:100;suffix:"%";value:settingsDialog.draft.globalVolume ?? 100;onEdited:value=>settingsDialog.draft=Object.assign({},settingsDialog.draft,{globalVolume:value})}
                    Rectangle {Layout.fillWidth:true;Layout.preferredHeight:1;color:root.divider}
                    Text {Layout.fillWidth:true;text:"Wallpaper audio plays while the desktop or WaniEngine has focus. Video keeps animating when another application is focused.";color:root.textMuted;font.pixelSize:13;wrapMode:Text.WordWrap}
                    Text {Layout.fillWidth:true;text:"Use the Audio switch in the wallpaper toolbar. Preview audio has its own mute control.";color:root.textDisabled;font.pixelSize:12;wrapMode:Text.WordWrap}
                    Item {Layout.fillHeight:true}
                }
                ColumnLayout {
                    spacing:16
                    Text {text:"Startup and window";color:root.textMain;font.pixelSize:16;font.weight:Font.DemiBold}
                    Toggle_redesign {Layout.fillWidth:true;text:"Start in the tray when signing in";checked:!!settingsDialog.draft.startWithSession;onToggled:settingsDialog.draft=Object.assign({},settingsDialog.draft,{startWithSession:checked})}
                    Toggle_redesign {Layout.fillWidth:true;text:"Restore saved wallpaper and characters on launch";checked:!!settingsDialog.draft.restoreOnLaunch;onToggled:settingsDialog.draft=Object.assign({},settingsDialog.draft,{restoreOnLaunch:checked})}
                    Rectangle {Layout.fillWidth:true;Layout.preferredHeight:1;color:root.divider}
                    Toggle_redesign {Layout.fillWidth:true;text:"Keep running in the tray when the window closes";checked:!!settingsDialog.draft.closeToTray;onToggled:settingsDialog.draft=Object.assign({},settingsDialog.draft,{closeToTray:checked})}
                    Toggle_redesign {Layout.fillWidth:true;text:"Animate selected previews";checked:!!settingsDialog.draft.animatePreview;onToggled:settingsDialog.draft=Object.assign({},settingsDialog.draft,{animatePreview:checked})}
                    Item {Layout.fillHeight:true}
                }
                ColumnLayout {
                    spacing:18
                    Text {text:"Your library";color:root.textMain;font.pixelSize:16;font.weight:Font.DemiBold}
                    Text {text:root.assets.length+" items · "+root.assets.filter(asset=>asset.kind==="wallpaper").length+" wallpapers · "+root.assets.filter(asset=>asset.kind==="character").length+" characters";color:root.textMuted;font.pixelSize:13}
                    Text {Layout.fillWidth:true;text:"Your installed collection is available in this preview. Original media and installed settings are preserved; preview changes are stored separately.";color:root.textDisabled;font.pixelSize:12;wrapMode:Text.WordWrap}
                    Text {Layout.fillWidth:true;text:sceneStore.libraryDirectory();color:root.textMuted;font.pixelSize:12;wrapMode:Text.WrapAnywhere}
                    Button_redesign {text:"Open WaniEngine videos folder";onClicked:sceneStore.openFolder("file://"+sceneStore.libraryDirectory())}
                    Item {Layout.fillHeight:true}
                }
                ColumnLayout {
                    spacing:18
                    Text {text:"WaniEngine";color:root.textMain;font.pixelSize:24;font.weight:Font.DemiBold}
                    Text {text:"Wallpapers and desktop characters for KDE Plasma.";color:root.textMuted;font.pixelSize:14}
                    Text {text:"Rust application · Qt 6 · Wayland";color:root.textDisabled;font.pixelSize:12}
                    Text {Layout.fillWidth:true;text:"Library and playlist storage run in Rust. Native Qt services handle desktop integration, multimedia and click-through overlays.";color:root.textDisabled;font.pixelSize:12;wrapMode:Text.WordWrap}
                    Item {Layout.fillHeight:true}
                }
            }
        }
    }


    Dialog_redesign {
        id: playlistDialog
        objectName: "playlistDialog"
        modal: true
        anchors.centerIn: Overlay.overlay
        width: 560; height: 440
        title: "Manage playlist — " + root.playlistName
        standardButtons: Dialog.Ok | Dialog.Cancel
        property var draftUrls: []
        property int intervalMinutes: 30
        property bool randomOrder: false
        onOpened: {
            draftUrls=root.playlist.slice(0)
            intervalMinutes=root.settings.playlistInterval || 30
            randomOrder=!!root.settings.playlistRandom
        }
        function moveItem(index, offset) {
            const next=draftUrls.slice(0), target=index+offset
            if(target<0 || target>=next.length) return
            const swap=next[target];next[target]=next[index];next[index]=swap;draftUrls=next
        }
        onAccepted: {
            root.playlist=draftUrls.slice(0);root.playlistDirty=true
            if(!root.playlist.length) root.playlistRunning=false
            root.settings=Object.assign({},root.settings,{playlistInterval:intervalMinutes,playlistRandom:randomOrder})
            root.saveState()
        }
        contentItem: ColumnLayout {
            spacing: 16
            Text { text: "Reorder or remove entries. Your files remain in the library."; color: root.textMuted; font.pixelSize: 11; Layout.fillWidth: true; wrapMode: Text.WordWrap }
            ListView {
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                model: playlistDialog.draftUrls
                ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }
                delegate: RowLayout {
                    required property string modelData
                    required property int index
                    width: ListView.view.width; height: 52
                    spacing: 12
                    property var entry: root.assets.find(item=>item.url===modelData)
                    Text { text: parent.entry ? parent.entry.name : root.fileName(parent.modelData); color: root.textMain; font.pixelSize: 11; Layout.fillWidth: true; elide: Text.ElideMiddle }
                    Button_redesign { text: "↑"; enabled: index>0; onClicked: playlistDialog.moveItem(index,-1) }
                    Button_redesign { text: "↓"; enabled: index<playlistDialog.draftUrls.length-1; onClicked: playlistDialog.moveItem(index,1) }
                    Button_redesign { text: "×"; onClicked: playlistDialog.draftUrls=playlistDialog.draftUrls.filter(url=>url!==modelData) }
                }
                Text { anchors.centerIn: parent; visible: !playlistDialog.draftUrls.length; text: "This playlist is empty."; color: root.textMuted; font.pixelSize: 11 }
            }
            RowLayout {
                Layout.fillWidth: true
                Text { text: "Rotate every (min)"; color: root.textMuted; font.pixelSize: 11 }
                SpinBox_redesign { from: 1; to: 1440; value: playlistDialog.intervalMinutes; onValueModified: playlistDialog.intervalMinutes=value }
                Item { Layout.fillWidth: true }
                Toggle_redesign { Layout.preferredWidth:112; text: "Shuffle"; checked: playlistDialog.randomOrder; onToggled: playlistDialog.randomOrder=checked }
            }
            Text {
                Layout.fillWidth: true
                text: "Use Play/Pause in the bottom bar to control rotation. Save a named playlist or load an existing one."
                color: root.textMuted; font.pixelSize: 11; wrapMode: Text.WordWrap
            }
            Text {
                Layout.fillWidth: true
                text: "Playlist folder: " + decodeURIComponent(String(playlistStore.directory()).replace("file://", ""))
                color: root.textDisabled; font.pixelSize: 10; wrapMode: Text.WrapAnywhere
            }
        }
    }

    Dialog_redesign {
        id: removeDialog
        modal: true
        anchors.centerIn: Overlay.overlay
        width: 420
        height: 190
        title: "Remove from library"
        standardButtons: Dialog.Cancel
        contentItem: ColumnLayout {
            Text {
                Layout.fillWidth: true
                text: root.selectedAsset
                      ? "Remove “" + root.selectedAsset.name + "” from the library?"
                      : "Remove this content from the library?"
                color: root.textMain
                font.pixelSize: 12
                wrapMode: Text.WordWrap
            }
            Text {
                Layout.fillWidth: true
                text: "The original file will be preserved."
                color: root.textMuted
                font.pixelSize: 11
                wrapMode: Text.WordWrap
            }
            Button_redesign {
                Layout.alignment: Qt.AlignRight
                text: "Remove"
                onClicked: if (root.selectedAsset) root.removeAsset(root.selectedUrl)
                background: Rectangle { color: "#FB7185"; radius: 10 }
                contentItem: Text { text: parent.text; color: "#F4F2FA"; font.pixelSize: 12; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter; leftPadding: 10; rightPadding: 10 }
            }
        }
    }
}
