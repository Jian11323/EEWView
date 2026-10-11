{
  description = "EEW View — earthquake early-warning desktop client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        version = "1.1.0";
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "eew-view";
          inherit version;
          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            makeWrapper
            copyDesktopItems
            python3
          ];

          buildInputs = with pkgs; [
            alsa-lib
            libxkbcommon
            xorg.libX11
            xorg.libXcursor
            xorg.libXrandr
            xorg.libXi
            libGL
            wayland
            gtk3
            openssl
          ];

          cargoBuildFlags = [ "-p" "jian-app" ];
          doCheck = false;

          postInstall = ''
            mkdir -p $out/share/eew-view
            cp -r assets config geodata $out/share/eew-view/
            rm -rf $out/share/eew-view/assets/travel/raw || true
            rm -f $out/share/eew-view/config/local.toml || true
            rm -f $out/share/eew-view/assets/lxgw-wenkai-v1.522/LXGWWenKai-Light.ttf || true
            rm -f $out/share/eew-view/assets/lxgw-wenkai-v1.522/LXGWWenKai-Medium.ttf || true
            rm -f $out/share/eew-view/assets/lxgw-wenkai-v1.522/LXGWWenKaiMono-*.ttf || true

            if [ -f assets/brand/app.png ]; then
              install -Dm644 assets/brand/app.png \
                $out/share/icons/hicolor/256x256/apps/eew-view.png
            fi

            makeWrapper $out/bin/jian $out/bin/eew-view \
              --set EEWVIEW_ROOT $out/share/eew-view \
              --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath [
                pkgs.alsa-lib
                pkgs.libxkbcommon
                pkgs.libGL
                pkgs.wayland
                pkgs.gtk3
              ]}"
          '';

          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "eew-view";
              desktopName = "EEW View";
              comment = "Earthquake early-warning monitor";
              exec = "eew-view";
              icon = "eew-view";
              categories = [ "Science" "Network" ];
              terminal = false;
            })
          ];

          meta = with pkgs.lib; {
            description = "EEW View desktop client";
            homepage = "https://github.com/Jian11323/EEWView";
            license = licenses.asl20;
            platforms = platforms.linux;
            mainProgram = "eew-view";
          };
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/eew-view";
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            pkg-config
            alsa-lib
            gtk3
            libxkbcommon
          ];
        };
      });
}
