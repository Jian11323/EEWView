{
  description = "StarQuake — earthquake early-warning desktop client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        version = "2.0.0";
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "starquake";
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
            mkdir -p $out/share/starquake
            cp -r media icon config geodata $out/share/starquake/
            rm -rf $out/share/starquake/media/travel/raw || true
            rm -f $out/share/starquake/config/local.toml || true
            rm -f $out/share/starquake/media/fonts/lxgw-wenkai-v1.522/LXGWWenKai-Light.ttf || true
            rm -f $out/share/starquake/media/fonts/lxgw-wenkai-v1.522/LXGWWenKai-Medium.ttf || true
            rm -f $out/share/starquake/media/fonts/lxgw-wenkai-v1.522/LXGWWenKaiMono-*.ttf || true

            if [ -f icon/brand/app.png ]; then
              install -Dm644 icon/brand/app.png \
                $out/share/icons/hicolor/256x256/apps/starquake.png
            fi

            makeWrapper $out/bin/starquake $out/bin/starquake-run \
              --set STARQUAKE_ROOT $out/share/starquake \
              --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath [
                pkgs.alsa-lib
                pkgs.libxkbcommon
                pkgs.libGL
                pkgs.wayland
                pkgs.gtk3
              ]}"
            ln -sf starquake-run $out/bin/starquake-app || true
          '';

          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "starquake";
              desktopName = "StarQuake";
              comment = "Earthquake early-warning monitor";
              exec = "starquake-run";
              icon = "starquake";
              categories = [ "Science" "Network" ];
              terminal = false;
            })
          ];

          meta = with pkgs.lib; {
            description = "StarQuake desktop client";
            homepage = "https://github.com/Jian11323/StarQuake";
            license = licenses.asl20;
            platforms = platforms.linux;
            mainProgram = "starquake-run";
          };
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/starquake-run";
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
