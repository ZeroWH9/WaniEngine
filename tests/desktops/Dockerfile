FROM archlinux:base
# This image supplies private desktop sessions, never the host display or bus.
RUN pacman -Syu --noconfirm --needed gnome-shell gnome-session gnome-settings-daemon gjs xorg-server-xvfb xfce4-session xfwm4 xfdesktop xfce4-settings dbus mesa libglvnd ffmpeg sdl2-compat libxkbcommon libxcursor libxrandr libxi python python-pillow pulseaudio xorg-xauth xorg-xrandr xorg-xdpyinfo xterm && pacman -Scc --noconfirm
RUN dbus-uuidgen --ensure=/etc/machine-id && mkdir -p /var/lib/dbus && ln -sf /etc/machine-id /var/lib/dbus/machine-id
RUN useradd -m -u 1000 wani
USER wani
ENV HOME=/home/wani LIBGL_ALWAYS_SOFTWARE=1
WORKDIR /workspace
