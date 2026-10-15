# 可选：从预构建 Linux 便携包安装（无需本机编译）。
# 用法（在已解压的 tar 旁，或改 src）：
#   nix-env -if packaging/nixos/default.nix
{ pkgs ? import <nixpkgs> { } }:

pkgs.stdenv.mkDerivation rec {
  pname = "eew-view";
  version = "1.4.1";

  # 将 CI / pack_linux 产物放到同目录并改名，或改下面 src
  src = ./EEWView_Beta_${version}_linux_x86_64.tar.gz;

  nativeBuildInputs = [ pkgs.makeWrapper ];

  installPhase = ''
    mkdir -p $out/share/eew-view $out/bin
    # tar 顶层为 EEWView/
    cp -r EEWView/* $out/share/eew-view/
    chmod +x $out/share/eew-view/eewview
    makeWrapper $out/share/eew-view/eewview $out/bin/eew-view \
      --set EEWVIEW_ROOT $out/share/eew-view
    makeWrapper $out/share/eew-view/eewview $out/bin/eewview \
      --set EEWVIEW_ROOT $out/share/eew-view

    mkdir -p $out/share/applications
    cat > $out/share/applications/eew-view.desktop <<EOF
[Desktop Entry]
Type=Application
Name=EEW View
Comment=Earthquake early-warning monitor
Exec=eew-view
Icon=eew-view
Terminal=false
Categories=Science;Network;
EOF
    if [ -f $out/share/eew-view/icon/brand/app.png ]; then
      mkdir -p $out/share/icons/hicolor/256x256/apps
      cp $out/share/eew-view/icon/brand/app.png \
        $out/share/icons/hicolor/256x256/apps/eew-view.png
    fi
  '';

  meta = with pkgs.lib; {
    description = "EEW View (prebuilt portable)";
    homepage = "https://github.com/Jian11323/EEWView";
    license = licenses.asl20;
    platforms = [ "x86_64-linux" ];
    mainProgram = "eew-view";
  };
}
