# The interface in English, the language every other file translates. `tr!` checks its ids and
# arguments against this file. One section per part of the app.


## browse
home-title = Home
home-my-media = My Media
home-continue-watching = Continue Watching
home-next-up = Next Up
home-favorites = Favorites
home-latest = Latest { $library }
library-empty = Your server has no libraries yet.
library-title = Libraries
library-count-movies = { $count ->
    [one] { $shown } movie
   *[other] { $shown } movies
    }
library-count-shows = { $count ->
    [one] { $shown } show
   *[other] { $shown } shows
    }
library-count-collections = { $count ->
    [one] { $shown } collection
   *[other] { $shown } collections
    }
library-count-playlists = { $count ->
    [one] { $shown } playlist
   *[other] { $shown } playlists
    }
library-count-items = { $count ->
    [one] { $shown } item
   *[other] { $shown } items
    }
library-sort-name = Name
library-sort-added = Date added
library-sort-released = Release date
library-sort-rating = Rating
library-sort-runtime = Runtime
library-sort-random = Random
library-progress-any = Any progress
library-progress-unwatched = Unwatched
library-progress-watched = Watched
library-progress-in-progress = In progress
library-resolution-any = Any resolution
library-filter-sort = Sort
library-filter-progress = Progress
library-filter-rating = Rating
library-filter-year = Year
library-filter-resolution = Resolution
library-filter-filters = Filters
library-filter-clear = Clear filters
library-filter-values = Genres and tags
library-filter-order = Order
library-any-rating = Any rating
library-any-year = Any year
library-shuffle-again = Shuffle again
library-sort-ascending = Sort ascending
library-sort-descending = Sort descending
library-jump-to = Jump to { $letter }
library-nothing-matches = Nothing matches these filters.
library-nothing-yet = Nothing here yet.
library-item-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
    }
library-values-chosen-items = { $chosen } chosen · { $items }
library-values-chosen = { $chosen } chosen
library-values-none-match = No genres or tags match “{ $search }”.
library-values-genres = Genres
library-values-tags = Tags
library-values-hint-touch = Tap once to include, twice to exclude, long-press to clear.
library-values-hint-mouse = Click once to include, twice to exclude, right-click to clear.
library-values-find = Find a genre or tag
library-values-not = Not { $value }
library-values-count = { $count ->
    [one] { $count } filter
   *[other] { $count } filters
    }
library-values-two = { $first }, { $second }
library-done = Done
library-clear = Clear
search-title = Search
search-type-to-search = Type in the search box to search.
search-no-results = No results.
search-movies = Movies
search-shows = Shows
search-episodes = Episodes
item-seasons = Seasons
item-in-this-collection = In this collection
item-playlist = Playlist
item-episodes = Episodes
item-episode-count = { $count ->
    [one] { $count } episode
   *[other] { $count } episodes
    }
item-to-watch = { $count } to watch
item-watched = Watched
item-no-episodes = No episodes in this season.
item-chapters = Chapters
item-cast-and-crew = Cast & crew
item-extras = Extras
item-more-like-this = More like this
item-play = Play
item-play-episode = Play { $episode }
item-play-all = Play all
item-play-from-beginning = Play from beginning
item-resume = Resume
item-resume-episode = Resume { $episode }
item-resume-from = Resume from { $time }
item-shuffle = Shuffle
item-trailer = Trailer
item-more = More
item-favorite = Favorite
item-from-start = From start
item-add-to-playlist = Add to playlist…
item-edit = Edit…
item-delete-playlist = Delete playlist…
item-version = Version
item-version-number = Version { $number }
item-nothing-to-play = Nothing here to play.
item-play-next = Play next
item-play-from-here = Play from here
item-add-to-queue = Add to queue
item-nothing-to-queue = Nothing here to add to the queue.
queue-played-next = { $name } plays next.
queue-added = Added { $name } to the queue.
queue-played-next-count = { $count ->
    [one] { $count } item from { $name } plays next.
   *[other] { $count } items from { $name } play next.
    }
queue-added-count = { $count ->
    [one] Added { $count } item from { $name } to the queue.
   *[other] Added { $count } items from { $name } to the queue.
    }
item-download = Download
item-downloading = Downloading
item-downloaded = Downloaded
item-download-all-episodes = Download all episodes
item-downloading-episodes = { $count ->
    [one] Downloading { $count } episode
   *[other] Downloading { $count } episodes
    }
item-downloaded-rest = { $count ->
    [one] { $count } episode downloaded. Download the rest
   *[other] { $count } episodes downloaded. Download the rest
    }
item-waiting-to-download = Waiting to download. Cancel download
item-waiting = Waiting
item-downloading-progress = Downloading, { $progress }. Cancel download
item-downloaded-size = Downloaded, { $size }. Remove download
item-download-failed = Download failed: { $message } Retry download
item-retry = Retry
item-cancel-download = Cancel download
item-remove-download = Remove download
item-retry-download = Retry download
item-left = { $time } left
item-less = Less
item-show-less = Show less
item-show-more = Show more
item-director = Director
item-writers = Writers
item-studio = Studio
item-playlist-not-yours = This playlist belongs to someone else; it can't be changed.
widget-back = Back
widget-cancel = Cancel
widget-retry = Retry
widget-scroll-left = Scroll left
widget-scroll-right = Scroll right
widget-move-up = Move up (Alt+Up)
widget-move-down = Move down (Alt+Down)
widget-remove-from-playlist = Remove from playlist (Delete)
widget-watched-key = Watched (W)
widget-copy-code = Copy code
widget-copied = Copied
widget-error = Error
widget-information = Information
widget-success = Success
widget-decrease = Decrease { $label }
widget-increase = Increase { $label }
widget-reset = Reset { $label }
widget-chapter-number = Chapter { $number }
widget-unwatched = { $count } unwatched
widget-included = { $label }, included
widget-excluded = { $label }, excluded

## player
player-back = Back (Esc)
player-starting = Starting playback…
player-play = Play (Space)
player-pause = Pause (Space)
player-back-10s = Back 10 s (Left)
player-forward-10s = Forward 10 s (Right)
player-mute = Mute (M)
player-unmute = Unmute (M)
player-fullscreen = Fullscreen (F)
player-exit-fullscreen = Exit fullscreen (F)
player-settings = Playback settings
player-subtitles = Subtitles
player-audio = Audio
player-chapters = Chapters
player-queue = Queue (Q)
player-queue-empty = Nothing is queued after this.
player-seek = Seek
player-volume = Volume
player-cast = Cast to a TV
player-cast-to-device = Casting to { $device }
player-playing-on = Playing on { $device }
player-reconnecting-to = Reconnecting to { $device }…
player-resumed-from = Resumed from { $time }
player-start-over = Start over
player-up-next = Up next
player-play-now = Play now
player-cancel = Cancel
player-chapter-number = Chapter { $number }
player-previous = Previous (Shift+P)
player-previous-episode = Previous episode (Shift+P)
player-previous-movie = Previous movie (Shift+P)
player-next = Next (Shift+N)
player-next-episode = Next episode (Shift+N)
player-next-movie = Next movie (Shift+N)
player-version = Version
player-version-number = Version { $number }
player-tab-general = General
player-no-audio-tracks = No audio tracks
player-no-subtitles = No subtitles
player-speed = Speed
player-size = Size
player-delay = Delay
player-normal-in-syncplay = Normal in SyncPlay
player-badge-syncplay = SyncPlay
player-badge-transcoding = Transcoding
player-sheet-settings = Settings
player-close = Close
player-subtitles-off = Off
player-track-number = Track { $index }
player-skip-intro = Skip intro
player-skip-recap = Skip recap
player-skip-preview = Skip preview
player-skip-ad = Skip ad
player-skip-credits = Skip credits
player-restarting = The player stopped unexpectedly. Restarting it…
player-reconnecting = Lost the connection to the server. Reconnecting…
player-crashing = The player keeps crashing on this item. The log may say why.
player-connection-lost = Lost the connection to the server. Try again when it's back.
player-not-streamable = The server can't stream this item to this app.
player-server-refused = The server won't play this item ({ $code }).
player-no-media = The server returned no playable media.
player-start-failed = Couldn't start the player: { $error }
player-mpv-android = mpv couldn't start. Pick Media3 as the player in Settings › Playback.
player-mpv-missing-macos = Video playback needs libmpv, which wasn't found. Install mpv with Homebrew (brew install mpv).
player-mpv-missing-windows = Video playback needs libmpv, which wasn't found. libmpv-2.dll is missing next to tabbyfin.exe; reinstall the app.
player-mpv-missing-linux = Video playback needs libmpv, which wasn't found. Install libmpv (the libmpv2 package on Debian and Ubuntu).
player-download-path = The downloaded file's path isn't valid: { $path }
player-stream-open-failed = The player couldn't open the stream: { $error }
player-play-failed = The player couldn't play this file: { $error }

## settings
settings-title = Settings
settings-store-device = This device
settings-store-server = Jellyfin server
settings-section-appearance = Appearance
settings-section-appearance-summary = Theme, text size, animations and language
settings-section-playback = Playback
settings-section-playback-summary = Next episode, intros, quality and the player
settings-section-subtitles = Subtitles
settings-section-subtitles-summary = Colour, style and size
settings-section-downloads = Downloads
settings-section-downloads-summary = Wi-Fi only, and where they're kept
settings-section-storage = Storage
settings-section-storage-summary = Image cache and logs
settings-section-about = About
settings-section-about-summary = Version and updates
settings-section-account = Account
settings-section-account-summary = Who's signed in, and the server
settings-section-languages = Languages
settings-section-languages-summary = Audio and subtitle tracks
settings-section-home = Home
settings-section-home-summary = What Home's rows show
settings-language = Language
settings-language-description = The app's language. Applies at once.
settings-language-unset = the system language
settings-language-system = Follow the system ({ $language })
settings-theme = Theme
settings-theme-description = Dark, light, or the desktop's colour scheme. Applies at once.
settings-theme-unset = dark
settings-theme-system = Follow the system
settings-theme-light = Light
settings-theme-dark = Dark
settings-text-size = Text size
settings-text-size-description = Scales the app's text and controls, not its layout. Applies at once.
settings-text-size-unset = the system text size
settings-text-size-system = Follow the system ({ $scale }%)
settings-animations = Animations
settings-animations-description = Animations and smooth scrolling. Applies at once.
settings-animations-unset = on
settings-tracks-note = Which audio and subtitle tracks an item starts with is kept on the server, for every app you use.
settings-autoplay = Play the next episode automatically
settings-autoplay-description = When an episode ends, so the player stays open. From the next item on.
settings-autoplay-unset = on
settings-skip-intro = Skip intros and recaps automatically
settings-skip-intro-description = Each intro or recap once as it starts, never in a SyncPlay group. From the next item on.
settings-skip-intro-unset = off
settings-bitrate = Max streaming bitrate
settings-bitrate-description = Videos above this bitrate are transcoded by the server. Lower it for slow connections. Automatic measures the connection before playing (at most every 10 minutes) and uses 80% of it, keeping the cap for when it's turned off. Read when the next item starts.
settings-bitrate-unset = no limit
settings-bitrate-auto-measured = Automatic (measured)
settings-bitrate-auto = Automatic
settings-bitrate-unlimited = No limit
settings-cast-bitrate = Chromecast bitrate
settings-cast-bitrate-description = The most a Chromecast or Google TV streams at when you cast to it; the server transcodes anything above it. Automatic leaves it to the TV, which measures its connection. Sent with the next command to the TV, which keeps it until it's changed.
settings-cast-bitrate-unset = automatic
settings-player = Player
settings-player-description = Media3 plays with Android's own decoders, keeps HDR and uses the least battery. mpv plays more kinds of files, and shows styled (ASS) subtitles as they were made. Read when the next item starts.
settings-player-unset = Media3
settings-hevc = Ask for HEVC when the server transcodes
settings-hevc-description = HEVC needs half the bandwidth of H.264 for the same picture, but is much harder for the server to encode. Used only if the server allows it (Dashboard › Playback › Transcoding). Read when the next item starts.
settings-hevc-unset = off
settings-subtitles-note = How text subtitles such as SRT look; styled (ASS) subtitles keep their own look and only change size. The player's settings menu also changes the size while watching.
settings-subtitle-tracks-note = Which subtitles turn on by themselves, and in which language, is kept on the server.
settings-subtitle-color = Colour
settings-subtitle-color-description = Of text subtitles. Read when the next item starts.
settings-subtitle-color-unset = white
settings-color-white = White
settings-color-yellow = Yellow
settings-subtitle-style = Style
settings-subtitle-style-description = An outline, or a translucent dark box, behind text subtitles. Read when the next item starts.
settings-subtitle-style-unset = outline
settings-style-outline = Outline
settings-style-box = Dark box
settings-subtitle-size = Size
settings-subtitle-size-description = Shared with the player's settings menu. Goes to the playback manager at once, so a running session changes with it.
settings-subtitle-size-unset = 100 %
settings-wifi-only = Download over Wi-Fi only
settings-wifi-only-description = On mobile data, downloads wait for Wi-Fi, and one under way stops and carries on from where it was once Wi-Fi is back. Applies at once.
settings-wifi-only-unset = on
settings-downloads-folder = Downloads folder
settings-downloads-folder-description = The app's own folder on the phone's shared storage. Uninstalling the app deletes it.
settings-image-cache = Image cache
settings-image-cache-description = Posters and backdrops are kept here and reload as pages ask for them again.
settings-logs = Logs
settings-logs-description = A log file is written each day. Attach it to bug reports.
settings-version = Version
settings-check-for-updates = Tell me when a new version is out
settings-check-for-updates-description = Checks the GitHub releases once a day, when the app starts.
settings-check-for-updates-unset = on
settings-user = User
settings-server = Server
settings-address = Address
settings-server-version = Jellyfin version
settings-languages-note = The server picks the tracks an item starts with from these. The player's menus change them while watching. From the next item on.
settings-audio-language = Audio language
settings-audio-language-description = The audio track in this language, when the file has one.
settings-play-default-audio = Play the default audio track
settings-play-default-audio-description = The track the file marks as default, whatever its language.
settings-remember-audio = Remember the audio track I pick
settings-remember-audio-description = A track picked by hand is picked again for the next episodes of the series.
settings-subtitle-language = Subtitle language
settings-subtitle-language-description = The language the subtitle mode looks for.
settings-subtitle-mode = Subtitle mode
settings-subtitle-mode-description = Which subtitles turn on by themselves. Default: the ones the file marks as default or forced. Smart: the subtitle language's when the audio is in another. Only forced: forced ones only. Always: the subtitle language's whatever the audio. None: none until one is picked.
settings-mode-default = Default
settings-mode-smart = Smart
settings-mode-only-forced = Only forced
settings-mode-always = Always
settings-mode-none = None
settings-remember-subtitles = Remember the subtitles I pick
settings-remember-subtitles-description = Subtitles picked by hand, or turned off, are picked again for the next episodes of the series.
settings-hide-played-in-latest = Leave watched items out of Latest
settings-hide-played-in-latest-description = Home's Latest rows show only what you haven't watched. Home reloads when it's saved.
settings-home-sections-note = Which sections Home shows, and the libraries in them, follow your Jellyfin profile (Settings › Home in the web client).
settings-no-match = No settings match “{ $query }”.
settings-kept-on-device = Kept on this computer, for every account signed in here.
settings-kept-in-account = Kept in { $user }'s account on the server, so the web client and other Jellyfin apps share them.
settings-kept-in-account-on = Kept in { $user }'s account on { $server }, so the web client and other Jellyfin apps share them.
settings-saving = Saving…
settings-saved-to-server = Saved to the server
settings-signed-in-as = Signed in as { $user } on { $server }
settings-key-config = { $key } in the config file
settings-key-config-unset = { $key } in the config file · unset: { $unset }
settings-key-profile = { $key } in your Jellyfin profile
settings-reset-default = Back to the default
settings-reset-default-tooltip = Back to the default: { $default }
settings-on = On
settings-off = Off
settings-clear = Clear
settings-open-folder = Open folder
settings-image-cache-cleared = Image cache cleared.
settings-image-cache-failed = Couldn't clear the image cache. Close anything using its folder and try again.
settings-logs-folder-failed = Couldn't open the logs folder ({ $path }).
settings-save-failed = Couldn't save your settings to the server. They're back as the server has them.
settings-load-failed = Couldn't load your settings from the server.
settings-loading = Loading your settings from the server…
settings-retry = Retry
settings-any-language = Any language

## shell

shell-home = Home
shell-libraries = Libraries
shell-back = Back
shell-settings = Settings
shell-syncplay = Watch together (SyncPlay)
shell-search = Search
shell-search-page = Search { $page }
shell-clear-search = Clear search
menu-filter = Filter
menu-no-matches = Nothing matches “{ $search }”.
shell-account-on = { $name } on { $server }
shell-downloads = Downloads
shell-add-account = Add account…
shell-sign-out = Sign out

shell-syncplay-title = Watch together
shell-syncplay-description = Play in sync with others on this server.
shell-syncplay-empty = No one is watching together right now.
shell-syncplay-join-section = Join a group
shell-syncplay-new-group = New group
shell-syncplay-anything = Anything you play now plays for everyone here.
shell-syncplay-leave = Leave group
shell-syncplay-join-label = Join { $group }, with { $people }
shell-syncplay-join = Join
shell-people-nobody = nobody yet
shell-people-and = { $others } and { $last }
shell-group-status-playing = Playing
shell-group-status-waiting = Waiting for everyone
shell-group-status-paused = Paused
shell-group-status-idle = Nothing playing
shell-group-subtitle = { $people } · { $status }
shell-group-watching = Watching · { $count }
shell-group-you = you
shell-queue = Queue
shell-queue-repeat-off = Repeat is off
shell-queue-repeat-all = Repeating the queue
shell-queue-repeat-one = Repeating one
shell-queue-shuffle-on = Shuffle is on
shell-queue-shuffle-off = Shuffle is off
shell-queue-playing = { $name }, playing
shell-queue-remove = Remove (Delete)
shell-new-group-name = { $user }'s group

shell-offline = Offline
shell-offline-hint = The server can't be reached. Your downloads are here, and the app keeps trying.

shell-update-message = Version { $version } is out (this is { $current }).
shell-update-title = Update available
shell-update-download = Download
shell-back-online = You're back online.

shell-toast-close = Close
shell-toast-information = Information
shell-toast-success = Success
shell-toast-error = Error

shell-error-invalid-url = That doesn't look like a valid server address.
shell-error-timeout = The server took too long to respond.
shell-error-connect = Couldn't reach the server. Check the address and your connection.
shell-error-broken = The connection to the server broke off. It may be restarting.
shell-error-starting = The server is still starting up.
shell-error-expired = Your session has expired. Please sign in again.
shell-error-forbidden = This account isn't allowed to do that.
shell-error-not-found = The server couldn't find what was requested.
shell-error-decode = The server sent a response this app doesn't understand.
shell-error-stalled = The server sent nothing for 30 seconds.
shell-error-socket = The live connection to the server failed.
shell-error-status = The server returned an error ({ $status }).

login-cancel = Cancel
login-use-another-server = Use another server
login-min-version = Works with Jellyfin { $version } or newer
login-back-to-account = Back to your account
login-quick-connect-title = Approve this sign-in
login-quick-connect-subtitle = Enter this code in Quick Connect, in a Jellyfin app you're signed in to.
login-to-server = Sign in to { $server }
login-server-line = Jellyfin { $version } at { $host }
login-welcome-back = Welcome back
login-welcome-back-subtitle = Pick an account to carry on watching.
login-welcome = Welcome to Tabbyfin
login-welcome-subtitle = Let's find your Jellyfin server.
login-address-required = Enter your server's address.
login-username-required = Enter your username.
login-wrong-credentials = Wrong username or password.
login-quick-connect-off = Quick Connect is turned off on this server.
login-quick-connect-expired = The Quick Connect code expired. Try again.
login-reconnect = You'll be signed back in automatically as soon as the server answers.
login-remove-account = Remove this account
login-server-address = Server address
login-other-server = Or connect to another server
login-address-hint = jellyfin.example.com or 192.168.1.10:8096
login-connect = Connect
login-username = Username
login-password = Password
login-hide-password = Hide password
login-show-password = Show password
login-sign-in = Sign in
login-quick-connect = Use Quick Connect
login-waiting-approval = Waiting for approval…
login-no-version = The server didn't report its version.
login-version-too-old = This server runs Jellyfin { $version }. Tabbyfin needs { $needed } or newer.
login-server-starting = The server is starting up. Waiting…
login-not-jellyfin = That address doesn't look like a Jellyfin server.
login-unreachable = Couldn't reach the server.

dialog-sharing = Everyone on this server can see it
dialog-delete-note = The playlist is removed from the server for everyone it's shared with. What's in it stays in your libraries.
dialog-added = Added to { $name }.
dialog-created = Made the playlist { $name }.
dialog-already-there = { $item } is already in { $name }.
dialog-cant-change = You can't change { $name }; it belongs to someone else.
dialog-add-to-playlist = Add to playlist
dialog-add-subtitle = Pick one of your playlists, or type a name to make a new one.
dialog-loading-playlists = Loading playlists…
dialog-no-playlist-named = No playlist has that name.
dialog-no-playlists = No playlists yet. Type a name to make one.
dialog-edit-playlist = Edit playlist
dialog-name = Name
dialog-checking-sharing = Checking who can see it…
dialog-save = Save
dialog-deleted = Deleted { $name }.
dialog-delete-forbidden = { $name } belongs to someone else; it can't be deleted.
dialog-delete-title = Delete “{ $name }”?
dialog-delete = Delete
dialog-new-playlist = New playlist “{ $name }”
dialog-row-label = { $name }, { $detail }
dialog-item-count = { $count ->
    [one] { $digits } item
   *[other] { $digits } items
  }
dialog-playlist-forbidden = This playlist belongs to someone else; it can't be changed.

## downloads-and-casting

# A select's closing brace is indented: `i18n::tests` reads a line that starts at the left margin
# as a new message.

# The downloads page, and the platform's notice of the transfers (Android's notification).
downloads-title = Downloads
downloads-empty = Nothing downloaded yet. Download a movie or an episode from its page or its menu.
downloads-summary-done = { $count ->
    [one] { $count } downloaded, { $size }
   *[other] { $count } downloaded, { $size }
  }
downloads-summary-coming = { $count } to come
downloads-summary-failed = { $count ->
    [one] { $count } failed
   *[other] { $count } failed
  }
downloads-status-waiting-wifi = Waiting for Wi-Fi
downloads-status-waiting = Waiting to download
downloads-status-paused = Paused
downloads-status-progress = { $done } of { $total } · { $percent }%
downloads-status-so-far = { $done } so far
downloads-status-failed = Failed: { $message }
downloads-action-play = Play
downloads-action-remove = Remove download
downloads-action-retry = Retry download
downloads-action-remove-queued = Remove from downloads
downloads-action-cancel = Cancel download
downloads-activity-waiting-wifi = Waiting for Wi-Fi
downloads-activity-downloading = Downloading
downloads-activity-to-come = { $count ->
    [one] { $count } download to come
   *[other] { $count } downloads to come
  }
downloads-activity-progress = { $done } of { $total }
downloads-activity-so-far = { $done } so far
downloads-activity-more = { $count } more to come
downloads-toast-downloaded = Downloaded { $title }
downloads-toast-failed = Couldn't download { $title }: { $message }
downloads-error-not-downloadable = This item can't be downloaded from this server.
downloads-error-not-media = Only videos, and the series and seasons that hold them, can be downloaded.
downloads-error-stopped = the download stopped unexpectedly
downloads-error-folder = couldn't make its folder: { $error }
downloads-error-no-media = the server returned no playable media
downloads-error-write-item = couldn't write { $file }: { $error }
downloads-error-open = couldn't open the file: { $error }
downloads-error-restart = couldn't restart the file: { $error }
downloads-error-continue = couldn't continue the file: { $error }
downloads-error-write = couldn't write the file: { $error }
downloads-error-partial = only { $done } of { $total } bytes arrived
downloads-error-finish = couldn't finish the file: { $error }
downloads-error-no-space = there isn't enough free space: { $needed } is needed and { $free } is free

# Casting: the panel, the now-casting bar and the toasts.
cast-title = Casting
cast-status-casting = Casting
cast-status-reconnecting = Reconnecting
cast-panel-title = Cast to a TV
cast-panel-note = Play on a Chromecast or Google TV on this network. Your TV plays it; this app is the remote.
cast-looking = Looking for TVs on this network…
cast-section-network = On this network
cast-device-casting = { $device }, casting
cast-device-reconnect = Reconnect to { $device }
cast-device-cast = Cast to { $device }
cast-device-running = Jellyfin is running
cast-device-group = Group
cast-stop = Stop casting
cast-disconnect-leave = Disconnect, and leave it playing
cast-back-10s = Back 10 s
cast-forward-10s = Forward 10 s
cast-volume = TV volume
cast-stop-join-title = Stop casting to join?
cast-stop-create-title = Stop casting to start a group?
cast-stop-note = A SyncPlay group plays here, in step with the others, so casting to { $device } stops. What it plays stops too.
cast-bar-label-on = { $name }, on { $device }
cast-bar-label-reconnecting = { $name }, reconnecting to { $device }
cast-bar-play = Play
cast-bar-pause = Pause
cast-bar-reconnecting = Reconnecting to { $device }…
cast-bar-series = { $series } · on { $device }
cast-bar-on = on { $device }
cast-button = Cast to a TV
cast-button-casting = Casting (cast to a TV)
cast-button-off-in-group = Cast to a TV (off in a SyncPlay group)
cast-toast-casting = Casting to { $device }
cast-toast-play-failed = The TV couldn't play this ({ $code }).
cast-toast-unreachable = The TV couldn't reach the server at { $address }. It needs an address the TV can reach on this network.
cast-toast-lost = Lost the connection to { $device }.
cast-toast-stopped = { $device } stopped casting.
cast-toast-leave-group = Leave the SyncPlay group to cast to a TV.
cast-error-connect = Couldn't connect to { $device }.
cast-error-launch = { $device } couldn't start Jellyfin.
cast-error-wrong-app = { $device } started an app that isn't Jellyfin's receiver.
cast-unknown-error = unknown error

# SyncPlay: the group's updates, as toasts.
syncplay-joined = Joined the SyncPlay group { $group }
syncplay-left = Left { $group }
syncplay-gone = That SyncPlay group no longer exists.
syncplay-no-access = Someone in the group can't access this item.
syncplay-user-joined = { $user } joined
syncplay-user-left = { $user } left
syncplay-error = SyncPlay: { $message }
syncplay-load-failed = Couldn't load the group's item: { $message }
