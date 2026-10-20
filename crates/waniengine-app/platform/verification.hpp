#pragma once
class QObject;
class RustSceneStore;
class RustPlaylistStore;
class PlasmaIntegration;
class OverlayController;
class ActorAudioController;
class QString;
int verify_backend(QObject *root,RustSceneStore &scene,RustPlaylistStore &playlists,PlasmaIntegration &plasma,OverlayController &overlays,ActorAudioController &audio,const QString &fixtures);
