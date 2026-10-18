# 可选：从预构建 Linux 便携包安装（无需本机编译）。
# 用法（在已解压的 tar 旁，或改 src）：
#   nix-env -if packaging/nixos/default.nix
{ pkgs ? import <nixpkgs> { } }:

pkgs.stdenv.mkDerivation rec {
  pname = "starquake";
  version = "2.0.0";

  # 将 CI / pack_linux 产物放到同目录并改名，或改下面 src
  src = ./StarQuake_${version}_linux_x86_64.tar.gz;

  nativeBuildInputs = [ pkgs.makeWrapper ];

  installPhase = ''
    mkdir -p $out/share/starquake $out/bin
    # tar 顶层为 StarQuake/
    cp -r StarQuake/* $out/share/starquake/
    chmod +x $out/share/starquake/starquake
    makeWrapper $out/share/starquake/starquake $out/bin/starquake \
      --set STARQUAKE_ROOT $out/share/starquake
    makeWrapper $out/share/starquake/starquake $out/bin/starquake-run \
      --set STARQUAKE_ROOT $out/share/starquake

    mkdir -p $out/share/applications
    cat > $out/share/applications/starquake.desktop <<EOF
[Desktop Entry]
Type=Application
Name=StarQuake
Comment=Earthquake early-warning monitor
Exec=starquake
Icon=starquake
Terminal=false
Categories=Science;Network;
EOF
    if [ -f $out/share/starquake/icon/brand/app.png ]; then
      mkdir -p $out/share/icons/hicolor/256x256/apps
      cp $out/share/starquake/icon/brand/app.png \
        $out/share/icons/hicolor/256x256/apps/starquake.png
    fi
  '';

  meta = with pkgs.lib; {
    description = "StarQuake (prebuilt portable)";
    homepage = "https://github.com/Jian11323/StarQuake";
    license = licenses.asl20;
    platforms = [ "x86_64-linux" ];
    mainProgram = "starquake";
  };
}
