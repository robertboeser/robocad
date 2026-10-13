# Releasing robocad

1. Update `version` in the root `Cargo.toml` and move the "Unreleased" section of `CHANGELOG.md` under the new version.
2. Run the full check and the manual checklist in `docs/manual-test.md` on a release build.
3. Commit, then tag: `git tag v0.2.0 && git push origin v0.2.0`.
4. The **Release** workflow builds, tests and packages on all three systems and attaches the files to the GitHub release:
   - macOS: `robocad-<version>-macos.dmg` (universal binary, ad-hoc signed)
   - Linux: `robocad-<version>-linux-x86_64.tar.gz` (binaries, `.desktop` file, icon)
   - Windows: `robocad-<version>-windows-x86_64.zip`

## Local packages
- macOS: `scripts/package-macos.sh` (add `--universal` for Intel + Apple Silicon)
- Linux: `scripts/package-linux.sh`
- Windows: `pwsh scripts/package-windows.ps1`

## Signing and notarization (macOS, optional)
The script signs ad hoc, which is enough for local use. For public downloads without Gatekeeper
warnings, sign with a Developer ID certificate and notarize:

```sh
codesign --force --deep --options runtime --sign "Developer ID Application: <Name> (<TEAM>)" dist/robocad.app
xcrun notarytool submit dist/robocad-<version>-macos.dmg --keychain-profile <profile> --wait
xcrun stapler staple dist/robocad-<version>-macos.dmg
```

## Known limitations
- Double-clicking a `.rcad` file in Finder starts robocad but does not open the file yet (the
  windowing library does not forward the "open file" event); drag the file onto the window or use
  File > Open. On Linux and Windows, file arguments (`robocad file.rcad`) work.
- The Windows executable has no embedded icon yet.
