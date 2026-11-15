{
  description = "Pinned native GNOME Crown HUD verification";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/b6c8664de9b6cc07fe5666a29f91884ba81197c4";
  outputs =
    { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          gnome-shell
          gjs
          pipewire
          pulseaudio
          dbus
          glib
          gtk4
          nodejs
          zip
          gettext
          jq
          libadwaita
          xorg-server
          xauth
          nixfmt
          shellcheck
          shfmt
        ];
        __EGL_VENDOR_LIBRARY_FILENAMES = "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json";
        LOCALE_ARCHIVE = "${pkgs.glibcLocales}/lib/locale/locale-archive";
        LIBGL_ALWAYS_SOFTWARE = "1";
        GALLIUM_DRIVER = "llvmpipe";
        LIBGL_DRIVERS_PATH = "${pkgs.mesa}/lib/dri";
        GI_TYPELIB_PATH =
          "${pkgs.gnome-shell}/lib/gnome-shell/girepository-1.0:"
          + pkgs.lib.makeSearchPath "lib/girepository-1.0" (
            map pkgs.lib.getLib [
              pkgs.glib
              pkgs.gtk4
              pkgs.libadwaita
              pkgs.gdk-pixbuf
              pkgs.pango
              pkgs.harfbuzz
              pkgs.graphene
              pkgs.gobject-introspection
            ]
          );
        shellHook = ''
          export LD_LIBRARY_PATH="${pkgs.gnome-shell}/lib/gnome-shell''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        '';
        OPENLOGI_GNOME_EXTENSIONS_RESOURCE = "${pkgs.gnome-shell}/share/gnome-shell/org.gnome.Shell.Extensions.src.gresource";
        OPENLOGI_DBUS_SESSION_CONFIG = "${pkgs.dbus}/share/dbus-1/session.conf";
        OPENLOGI_PIPEWIRE_CONFIG_DIR = "${pkgs.pipewire}/share/pipewire";
        XDG_DATA_DIRS = pkgs.lib.makeSearchPath "share" [
          pkgs.gsettings-desktop-schemas
          pkgs.gnome-shell
          pkgs.gtk4
        ];
      };
    };
}
