# Submitting to the OpenAction catalogue

Checklist for listing this plugin in [OpenActionAPI/plugins](https://github.com/OpenActionAPI/plugins)
(what OpenDeck's plugin store reads). Its validator checks each item below.

1. **Repository sidebar description**, set exactly to:

   > OpenAction plugin for KDE Connect: media, volume, battery, notifications, calls and more on your phone, tablet or computer

   ```sh
   gh repo edit mroneleg/kde-connect-opendeck \
     --description "OpenAction plugin for KDE Connect: media, volume, battery, notifications, calls and more on your phone, tablet or computer" \
     --add-topic openaction
   ```

2. **`openaction` topic** on the repository (the command above adds it).

3. **A release** whose assets include `com.mroneleg.kdeconnect.streamDeckPlugin`
   (push a `v1.0.0` tag; the release workflow builds it).

4. **Catalogue entry**, added to `catalogue.json` in the *Native OpenAction plugins*
   section, sorted by repository URL (`https://github.com/mroneleg/...` goes after
   `https://github.com/mdvictor/...` and before `https://github.com/sjourdois/...`
   at the time of writing). The file uses tab indentation:

   ```json
   "com.mroneleg.kdeconnect": {
   	"name": "KDE Connect",
   	"author": "Mr. One Leg",
   	"repository": "https://github.com/mroneleg/kde-connect-opendeck",
   	"description": "OpenAction plugin for KDE Connect: media, volume, battery, notifications, calls and more on your phone, tablet or computer",
   	"flags": {
   		"ai_assisted": true
   	}
   }
   ```

   `name` and `author` must match `assets/manifest.json`; `description` must match
   the repository description exactly.

5. **Icon**: copy `assets/icons/plugin@4x.png` (288×288) to `icons/com.mroneleg.kdeconnect.png`
   in the catalogue repository. It must be a PNG of at least 144×144.
