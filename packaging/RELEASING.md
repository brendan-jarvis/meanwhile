# Releasing meanwhile (GitHub + AUR)

## GitHub release

```sh
cd ~/dev/meanwhile
git push                                  # push accumulated commits
git tag v0.4.0 && git push origin v0.4.0  # tag the release
```

## AUR (first time)

1. Create an AUR account at https://aur.archlinux.org and add your SSH key.
2. Fill the checksum and generate .SRCINFO:

```sh
cd ~/dev/meanwhile/packaging/aur
sha=$(curl -sL https://github.com/tomdavenport/meanwhile/archive/refs/tags/v0.4.0.tar.gz | sha256sum | cut -d' ' -f1)
sed -i "s/FILL_ON_RELEASE/$sha/" PKGBUILD
# ensure Cargo.lock is committed so --locked works in prepare()
makepkg --printsrcinfo > .SRCINFO
makepkg -si            # local test install: builds and installs the package
```

3. Publish:

```sh
git clone ssh://aur@aur.archlinux.org/meanwhile-rain.git /tmp/aur-meanwhile-rain
cp PKGBUILD .SRCINFO /tmp/aur-meanwhile-rain/
cd /tmp/aur-meanwhile-rain && git add -A && git commit -m "meanwhile-rain 0.4.0" && git push
```

Later releases: bump `pkgver`, redo the checksum + .SRCINFO, commit, push.
