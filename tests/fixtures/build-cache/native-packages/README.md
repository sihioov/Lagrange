# Native package identity fixture

These files are byte-exact stdout captured from the pinned C6 native builder at
commit `d701dae2016b0486e77776f0b2b081c76ad5ab2c` on Alpine 3.24.1 with APK
3.0.6-r0. The probe performed no project compilation.

Capture command for `apk-installed-json.stdout`:

```sh
apk query --installed --all-matches --format json \
  --fields name,version,provides,status '*'
```

`apk-info-vv.stdout` is the corresponding raw `apk info -vv` inventory retained
for evidence and benchmark metrics.

- `apk-installed-json.stdout`: SHA256
  `32b910c3133df4271cb4f5a08231f5c41349b52fddea8facf6ec1cd93e60a46d`
- `apk-info-vv.stdout`: SHA256
  `119064aed0d76b0c9d1b846db422c3d5f05b3cb8d7601de50b6647eafbd2699e`

The observed virtual dependency is exact: installed `postgresql18-dev` declares
`postgresql-dev` in `provides`. Tests must not infer that relationship from the
package-name prefix or from descriptive text.
