# La interfaz en español de España: una traducción de cada mensaje de `en/tabbyfin.ftl`, con las
# mismas secciones y argumentos.


## browse
home-title = Inicio
home-my-media = Mis medios
home-continue-watching = Seguir viendo
home-next-up = A continuación
home-favorites = Favoritos
home-latest = Novedades en { $library }
library-empty = Tu servidor aún no tiene bibliotecas.
library-title = Bibliotecas
library-count-movies = { $count ->
    [one] { $shown } película
   *[other] { $shown } películas
    }
library-count-shows = { $count ->
    [one] { $shown } serie
   *[other] { $shown } series
    }
library-count-collections = { $count ->
    [one] { $shown } colección
   *[other] { $shown } colecciones
    }
library-count-playlists = { $count ->
    [one] { $shown } lista
   *[other] { $shown } listas
    }
library-count-items = { $count ->
    [one] { $shown } elemento
   *[other] { $shown } elementos
    }
library-sort-name = Nombre
library-sort-added = Fecha de adición
library-sort-released = Fecha de estreno
library-sort-rating = Valoración
library-sort-runtime = Duración
library-sort-random = Aleatorio
library-progress-any = Cualquier progreso
library-progress-unwatched = Sin ver
library-progress-watched = Vistos
library-progress-in-progress = A medias
library-resolution-any = Cualquier resolución
library-filter-sort = Ordenar
library-filter-progress = Progreso
library-filter-rating = Valoración
library-filter-year = Año
library-filter-resolution = Resolución
library-filter-filters = Filtros
library-filter-clear = Quitar filtros
library-filter-values = Géneros y etiquetas
library-filter-order = Orden
library-any-rating = Cualquier valoración
library-any-year = Cualquier año
library-shuffle-again = Volver a mezclar
library-sort-ascending = Orden ascendente
library-sort-descending = Orden descendente
library-jump-to = Ir a { $letter }
library-nothing-matches = Nada coincide con estos filtros.
library-nothing-yet = Aún no hay nada aquí.
library-item-count = { $count ->
    [one] { $count } elemento
   *[other] { $count } elementos
    }
library-values-chosen-items = Elegidos: { $chosen } · { $items }
library-values-chosen = Elegidos: { $chosen }
library-values-none-match = Ningún género ni etiqueta coincide con «{ $search }».
library-values-genres = Géneros
library-values-tags = Etiquetas
library-values-hint-touch = Toca una vez para incluir, dos para excluir, y mantén pulsado para quitar.
library-values-hint-mouse = Haz clic una vez para incluir, dos para excluir, y clic derecho para quitar.
library-values-find = Buscar un género o etiqueta
library-values-not = Sin { $value }
library-values-count = { $count ->
    [one] { $count } filtro
   *[other] { $count } filtros
    }
library-values-two = { $first }, { $second }
library-done = Hecho
library-clear = Quitar
search-title = Buscar
search-type-to-search = Escribe en el cuadro de búsqueda para buscar.
search-no-results = No hay resultados.
search-movies = Películas
search-shows = Series
search-episodes = Episodios
item-seasons = Temporadas
item-in-this-collection = En esta colección
item-playlist = Lista de reproducción
item-episodes = Episodios
item-episode-count = { $count ->
    [one] { $count } episodio
   *[other] { $count } episodios
    }
item-to-watch = { $count } por ver
item-watched = Visto
item-no-episodes = No hay episodios en esta temporada.
item-chapters = Capítulos
item-cast-and-crew = Reparto y equipo
item-extras = Extras
item-more-like-this = Más como este
item-play = Reproducir
item-play-episode = Reproducir { $episode }
item-play-all = Reproducir todo
item-play-from-beginning = Reproducir desde el principio
item-resume = Reanudar
item-resume-episode = Reanudar { $episode }
item-resume-from = Reanudar desde { $time }
item-shuffle = Aleatorio
item-trailer = Tráiler
item-more = Más
item-favorite = Favorito
item-from-start = Desde el principio
item-add-to-playlist = Añadir a una lista…
item-edit = Editar…
item-delete-playlist = Eliminar lista…
item-version = Versión
item-version-number = Versión { $number }
item-nothing-to-play = No hay nada que reproducir aquí.
item-download = Descargar
item-downloading = Descargando
item-downloaded = Descargado
item-download-all-episodes = Descargar todos los episodios
item-downloading-episodes = { $count ->
    [one] Descargando { $count } episodio
   *[other] Descargando { $count } episodios
    }
item-downloaded-rest = { $count ->
    [one] { $count } episodio descargado. Descargar el resto
   *[other] { $count } episodios descargados. Descargar el resto
    }
item-waiting-to-download = Esperando para descargar. Cancelar descarga
item-waiting = En espera
item-downloading-progress = Descargando, { $progress }. Cancelar descarga
item-downloaded-size = Descargado, { $size }. Quitar descarga
item-download-failed = La descarga ha fallado: { $message } Reintentar descarga
item-retry = Reintentar
item-cancel-download = Cancelar descarga
item-remove-download = Quitar descarga
item-retry-download = Reintentar descarga
item-left = Quedan { $time }
item-less = Menos
item-show-less = Mostrar menos
item-show-more = Mostrar más
item-director = Dirección
item-writers = Guion
item-studio = Estudio
item-playlist-not-yours = Esta lista es de otra persona; no se puede cambiar.
widget-back = Atrás
widget-cancel = Cancelar
widget-retry = Reintentar
widget-scroll-left = Desplazar a la izquierda
widget-scroll-right = Desplazar a la derecha
widget-move-up = Subir (Alt+Arriba)
widget-move-down = Bajar (Alt+Abajo)
widget-remove-from-playlist = Quitar de la lista (Supr)
widget-watched-key = Visto (W)
widget-copy-code = Copiar código
widget-copied = Copiado
widget-error = Error
widget-information = Información
widget-warning = Advertencia
widget-success = Hecho
widget-decrease = Reducir { $label }
widget-increase = Aumentar { $label }
widget-reset = Restablecer { $label }
widget-chapter-number = Capítulo { $number }
widget-unwatched = { $count } sin ver
widget-included = { $label }, incluido
widget-excluded = { $label }, excluido

## player
player-back = Atrás (Esc)
player-starting = Iniciando la reproducción…
player-play = Reproducir (Espacio)
player-pause = Pausa (Espacio)
player-back-10s = Retroceder 10 s (Izquierda)
player-forward-10s = Avanzar 10 s (Derecha)
player-mute = Silenciar (M)
player-unmute = Activar sonido (M)
player-fullscreen = Pantalla completa (F)
player-exit-fullscreen = Salir de pantalla completa (F)
player-settings = Ajustes de reproducción
player-subtitles = Subtítulos
player-audio = Audio
player-chapters = Capítulos
player-seek = Posición
player-volume = Volumen
player-cast = Enviar a una TV
player-cast-to-device = Enviando a { $device }
player-playing-on = Reproduciendo en { $device }
player-reconnecting-to = Reconectando con { $device }…
player-resumed-from = Reanudado desde { $time }
player-start-over = Empezar de nuevo
player-up-next = A continuación
player-play-now = Reproducir ya
player-cancel = Cancelar
player-chapter-number = Capítulo { $number }
player-previous = Anterior (Mayús+P)
player-previous-episode = Episodio anterior (Mayús+P)
player-previous-movie = Película anterior (Mayús+P)
player-next = Siguiente (Mayús+N)
player-next-episode = Episodio siguiente (Mayús+N)
player-next-movie = Película siguiente (Mayús+N)
player-version = Versión
player-version-number = Versión { $number }
player-tab-general = General
player-no-audio-tracks = No hay pistas de audio
player-no-subtitles = No hay subtítulos
player-speed = Velocidad
player-size = Tamaño
player-delay = Retardo
player-normal-in-syncplay = Normal en SyncPlay
player-badge-syncplay = SyncPlay
player-badge-transcoding = Transcodificando
player-sheet-settings = Ajustes
player-close = Cerrar
player-subtitles-off = Desactivados
player-track-number = Pista { $index }
player-skip-intro = Saltar intro
player-skip-recap = Saltar resumen
player-skip-preview = Saltar avance
player-skip-ad = Saltar anuncio
player-skip-credits = Saltar créditos
player-restarting = El reproductor se ha detenido inesperadamente. Reiniciándolo…
player-reconnecting = Se ha perdido la conexión con el servidor. Reconectando…
player-crashing = El reproductor sigue fallando con este elemento. El registro puede decir por qué.
player-connection-lost = Se ha perdido la conexión con el servidor. Vuelve a intentarlo cuando se recupere.
player-not-streamable = El servidor no puede transmitir este elemento a esta aplicación.
player-server-refused = El servidor no quiere reproducir este elemento ({ $code }).
player-no-media = El servidor no ha devuelto ningún medio reproducible.
player-start-failed = No se ha podido iniciar el reproductor: { $error }
player-mpv-android = mpv no ha podido iniciarse. Elige Media3 como reproductor en Ajustes › Reproducción.
player-mpv-missing-macos = Para reproducir vídeo hace falta libmpv, que no se ha encontrado. Instala mpv con Homebrew (brew install mpv).
player-mpv-missing-windows = Para reproducir vídeo hace falta libmpv, que no se ha encontrado. Falta libmpv-2.dll junto a tabbyfin.exe; vuelve a instalar la aplicación.
player-mpv-missing-linux = Para reproducir vídeo hace falta libmpv, que no se ha encontrado. Instala libmpv (el paquete libmpv2 en Debian y Ubuntu).
player-download-path = La ruta del archivo descargado no es válida: { $path }
player-stream-open-failed = El reproductor no ha podido abrir la transmisión: { $error }
player-play-failed = El reproductor no ha podido reproducir este archivo: { $error }

## settings
settings-title = Ajustes
settings-store-device = Este dispositivo
settings-store-server = Servidor de Jellyfin
settings-section-appearance = Apariencia
settings-section-appearance-summary = Tema, tamaño del texto, animaciones e idioma
settings-section-playback = Reproducción
settings-section-playback-summary = Episodio siguiente, intros, calidad y el reproductor
settings-section-subtitles = Subtítulos
settings-section-subtitles-summary = Color, estilo y tamaño
settings-section-downloads = Descargas
settings-section-downloads-summary = Solo con wifi, y dónde se guardan
settings-section-storage = Almacenamiento
settings-section-storage-summary = Caché de imágenes y registros
settings-section-about = Acerca de
settings-section-about-summary = Versión y actualizaciones
settings-section-account = Cuenta
settings-section-account-summary = Quién ha iniciado sesión, y el servidor
settings-section-languages = Idiomas
settings-section-languages-summary = Pistas de audio y subtítulos
settings-section-home = Inicio
settings-section-home-summary = Qué muestran las filas de Inicio
settings-language = Idioma
settings-language-description = El idioma de la aplicación. Se aplica al momento.
settings-language-unset = el idioma del sistema
settings-language-system = Seguir el sistema ({ $language })
settings-theme = Tema
settings-theme-description = Oscuro, claro o el esquema de colores del escritorio. Se aplica al momento.
settings-theme-unset = oscuro
settings-theme-system = Seguir el sistema
settings-theme-light = Claro
settings-theme-dark = Oscuro
settings-text-size = Tamaño del texto
settings-text-size-description = Escala el texto y los controles de la aplicación, no su disposición. Se aplica al momento.
settings-text-size-unset = el tamaño del texto del sistema
settings-text-size-system = Seguir el sistema ({ $scale } %)
settings-animations = Animaciones
settings-animations-description = Animaciones y desplazamiento suave. Se aplica al momento.
settings-animations-unset = activadas
settings-tracks-note = Las pistas de audio y subtítulos con las que empieza un elemento se guardan en el servidor, para todas las aplicaciones que uses.
settings-autoplay = Reproducir el siguiente episodio automáticamente
settings-autoplay-description = Cuando termina un episodio, así el reproductor sigue abierto. A partir del siguiente elemento.
settings-autoplay-unset = activado
settings-skip-intro = Saltar intros y resúmenes automáticamente
settings-skip-intro-description = Cada intro o resumen una vez, al empezar, nunca en un grupo de SyncPlay. A partir del siguiente elemento.
settings-skip-intro-unset = desactivado
settings-bitrate = Tasa de bits máxima de transmisión
settings-bitrate-description = El servidor transcodifica los vídeos que superan esta tasa de bits. Bájala para conexiones lentas. Automática mide la conexión antes de reproducir (como mucho cada 10 minutos) y usa el 80 % de ella, y guarda el límite para cuando se desactive. Se lee cuando empieza el siguiente elemento.
settings-bitrate-unset = sin límite
settings-bitrate-auto-measured = Automática (medida)
settings-bitrate-auto = Automática
settings-bitrate-unlimited = Sin límite
settings-cast-bitrate = Tasa de bits de Chromecast
settings-cast-bitrate-description = Lo máximo a lo que transmite un Chromecast o Google TV cuando envías contenido; el servidor transcodifica lo que lo supere. Automática lo deja en manos de la TV, que mide su conexión. Se envía con la siguiente orden a la TV, que lo mantiene hasta que se cambie.
settings-cast-bitrate-unset = automática
settings-player = Reproductor
settings-player-description = Media3 reproduce con los decodificadores del propio Android, mantiene el HDR y gasta menos batería. mpv reproduce más tipos de archivo y muestra los subtítulos con estilo (ASS) tal como se hicieron. Se lee cuando empieza el siguiente elemento.
settings-player-unset = Media3
settings-hevc = Pedir HEVC cuando el servidor transcodifica
settings-hevc-description = HEVC necesita la mitad de ancho de banda que H.264 para la misma imagen, pero al servidor le cuesta mucho más codificarlo. Solo se usa si el servidor lo permite (Panel › Reproducción › Transcodificación). Se lee cuando empieza el siguiente elemento.
settings-hevc-unset = desactivado
settings-subtitles-note = Cómo se ven los subtítulos de texto como SRT; los subtítulos con estilo (ASS) mantienen su aspecto y solo cambian de tamaño. El menú de ajustes del reproductor también cambia el tamaño mientras ves algo.
settings-subtitle-tracks-note = Qué subtítulos se activan solos, y en qué idioma, se guarda en el servidor.
settings-subtitle-color = Color
settings-subtitle-color-description = De los subtítulos de texto. Se lee cuando empieza el siguiente elemento.
settings-subtitle-color-unset = blanco
settings-color-white = Blanco
settings-color-yellow = Amarillo
settings-subtitle-style = Estilo
settings-subtitle-style-description = Un contorno, o una caja oscura translúcida, detrás de los subtítulos de texto. Se lee cuando empieza el siguiente elemento.
settings-subtitle-style-unset = contorno
settings-style-outline = Contorno
settings-style-box = Caja oscura
settings-subtitle-size = Tamaño
settings-subtitle-size-description = Compartido con el menú de ajustes del reproductor. Pasa al gestor de reproducción al momento, así que una sesión en curso cambia con él.
settings-subtitle-size-unset = 100 %
settings-wifi-only = Descargar solo con wifi
settings-wifi-only-description = Con datos móviles, las descargas esperan al wifi, y la que esté en curso se detiene y continúa donde estaba cuando vuelva el wifi. Se aplica al momento.
settings-wifi-only-unset = activado
settings-downloads-folder = Carpeta de descargas
settings-downloads-folder-description = La carpeta propia de la aplicación en el almacenamiento compartido del teléfono. Desinstalar la aplicación la elimina.
settings-image-cache = Caché de imágenes
settings-image-cache-description = Los pósteres y fondos se guardan aquí y se vuelven a cargar cuando las páginas los piden de nuevo.
settings-logs = Registros
settings-logs-description = Cada día se escribe un archivo de registro. Adjúntalo a los informes de errores.
settings-version = Versión
settings-check-for-updates = Avisarme cuando salga una versión nueva
settings-check-for-updates-description = Consulta las versiones de GitHub una vez al día, al iniciar la aplicación.
settings-check-for-updates-unset = activado
settings-user = Usuario
settings-server = Servidor
settings-address = Dirección
settings-server-version = Versión de Jellyfin
settings-languages-note = El servidor elige las pistas con las que empieza un elemento a partir de estos ajustes. Los menús del reproductor las cambian mientras ves algo. A partir del siguiente elemento.
settings-audio-language = Idioma del audio
settings-audio-language-description = La pista de audio en este idioma, cuando el archivo la tiene.
settings-play-default-audio = Reproducir la pista de audio predeterminada
settings-play-default-audio-description = La pista que el archivo marca como predeterminada, sea cual sea su idioma.
settings-remember-audio = Recordar la pista de audio que elija
settings-remember-audio-description = Una pista elegida a mano se vuelve a elegir en los siguientes episodios de la serie.
settings-subtitle-language = Idioma de los subtítulos
settings-subtitle-language-description = El idioma que busca el modo de subtítulos.
settings-subtitle-mode = Modo de subtítulos
settings-subtitle-mode-description = Qué subtítulos se activan solos. Predeterminado: los que el archivo marca como predeterminados o forzados. Inteligente: los del idioma de los subtítulos cuando el audio está en otro. Solo forzados: solo los forzados. Siempre: los del idioma de los subtítulos, sea cual sea el audio. Ninguno: ninguno hasta que se elija uno.
settings-mode-default = Predeterminado
settings-mode-smart = Inteligente
settings-mode-only-forced = Solo forzados
settings-mode-always = Siempre
settings-mode-none = Ninguno
settings-remember-subtitles = Recordar los subtítulos que elija
settings-remember-subtitles-description = Los subtítulos elegidos a mano, o desactivados, se vuelven a elegir en los siguientes episodios de la serie.
settings-hide-played-in-latest = No mostrar lo ya visto en Novedades
settings-hide-played-in-latest-description = Las filas de Novedades de Inicio solo muestran lo que no has visto. Inicio se vuelve a cargar al guardarlo.
settings-home-sections-note = Qué secciones muestra Inicio, y las bibliotecas que contienen, siguen tu perfil de Jellyfin (Ajustes › Inicio en el cliente web).
settings-no-match = Ningún ajuste coincide con «{ $query }».
settings-kept-on-device = Se guarda en este ordenador, para todas las cuentas que inicien sesión aquí.
settings-kept-in-account = Se guarda en la cuenta de { $user } en el servidor, así el cliente web y las otras aplicaciones de Jellyfin lo comparten.
settings-kept-in-account-on = Se guarda en la cuenta de { $user } en { $server }, así el cliente web y las otras aplicaciones de Jellyfin lo comparten.
settings-saving = Guardando…
settings-saved-to-server = Guardado en el servidor
settings-signed-in-as = Sesión iniciada como { $user } en { $server }
settings-key-config = { $key } en el archivo de configuración
settings-key-config-unset = { $key } en el archivo de configuración · sin definir: { $unset }
settings-key-profile = { $key } en tu perfil de Jellyfin
settings-reset-default = Volver al valor predeterminado
settings-reset-default-tooltip = Volver al valor predeterminado: { $default }
settings-on = Activado
settings-off = Desactivado
settings-clear = Vaciar
settings-open-folder = Abrir carpeta
settings-image-cache-cleared = Se ha vaciado la caché de imágenes.
settings-image-cache-failed = No se ha podido vaciar la caché de imágenes. Cierra lo que esté usando su carpeta y vuelve a intentarlo.
settings-logs-folder-failed = No se ha podido abrir la carpeta de registros ({ $path }).
settings-save-failed = No se han podido guardar tus ajustes en el servidor. Han vuelto a como los tiene el servidor.
settings-load-failed = No se han podido cargar tus ajustes desde el servidor.
settings-loading = Cargando tus ajustes desde el servidor…
settings-retry = Reintentar
settings-any-language = Cualquier idioma

## shell

shell-home = Inicio
shell-libraries = Bibliotecas
shell-back = Atrás
shell-settings = Ajustes
shell-syncplay = Ver juntos (SyncPlay)
shell-search = Buscar
shell-search-page = Buscar en { $page }
shell-clear-search = Borrar búsqueda
menu-filter = Filtrar
menu-no-matches = Nada coincide con «{ $search }».
shell-account-on = { $name } en { $server }
shell-downloads = Descargas
shell-add-account = Añadir cuenta…
shell-sign-out = Cerrar sesión

shell-syncplay-title = Ver juntos
shell-syncplay-description = Reproduce en sincronía con otras personas de este servidor.
shell-syncplay-empty = Ahora mismo nadie está viendo nada en grupo.
shell-syncplay-join-section = Unirse a un grupo
shell-syncplay-new-group = Grupo nuevo
shell-syncplay-anything = Todo lo que reproduzcas ahora se reproduce para todos los de aquí.
shell-syncplay-leave = Salir del grupo
shell-syncplay-join-label = Unirse a { $group }, con { $people }
shell-syncplay-join = Unirse
shell-people-nobody = nadie todavía
shell-people-and = { $others } y { $last }
shell-group-status-playing = Reproduciendo
shell-group-status-waiting = Esperando a todos
shell-group-status-paused = En pausa
shell-group-status-idle = Nada en reproducción
shell-group-subtitle = { $people } · { $status }
shell-group-watching = Viendo · { $count }
shell-group-you = tú
shell-queue = Cola
shell-queue-repeat-off = Repetición desactivada
shell-queue-repeat-all = Repitiendo la cola
shell-queue-repeat-one = Repitiendo uno
shell-queue-shuffle-on = Aleatorio activado
shell-queue-shuffle-off = Aleatorio desactivado
shell-queue-playing = { $name }, en reproducción
shell-queue-remove = Quitar (Supr)
shell-new-group-name = Grupo de { $user }

shell-offline = Sin conexión
shell-offline-hint = No se puede contactar con el servidor. Tus descargas están aquí, y la aplicación sigue intentándolo.

shell-update-message = Ya está disponible la versión { $version } (esta es la { $current }).
shell-update-title = Actualización disponible
shell-update-download = Descargar
shell-back-online = Vuelves a tener conexión.

shell-toast-close = Cerrar
shell-toast-information = Información
shell-toast-success = Hecho
shell-toast-error = Error

shell-error-invalid-url = Eso no parece una dirección de servidor válida.
shell-error-timeout = El servidor ha tardado demasiado en responder.
shell-error-connect = No se ha podido contactar con el servidor. Comprueba la dirección y tu conexión.
shell-error-broken = Se ha cortado la conexión con el servidor. Puede que se esté reiniciando.
shell-error-starting = El servidor aún se está iniciando.
shell-error-expired = Tu sesión ha caducado. Vuelve a iniciar sesión.
shell-error-forbidden = Esta cuenta no tiene permiso para hacer eso.
shell-error-not-found = El servidor no ha encontrado lo que se ha pedido.
shell-error-decode = El servidor ha enviado una respuesta que esta aplicación no entiende.
shell-error-stalled = El servidor no ha enviado nada en 30 segundos.
shell-error-socket = Ha fallado la conexión en directo con el servidor.
shell-error-status = El servidor ha devuelto un error ({ $status }).

login-cancel = Cancelar
login-use-another-server = Usar otro servidor
login-min-version = Funciona con Jellyfin { $version } o posterior
login-back-to-account = Volver a tu cuenta
login-quick-connect-title = Aprueba este inicio de sesión
login-quick-connect-subtitle = Introduce este código en Conexión rápida, en una aplicación de Jellyfin en la que hayas iniciado sesión.
login-to-server = Iniciar sesión en { $server }
login-server-line = Jellyfin { $version } en { $host }
login-welcome-back = Hola de nuevo
login-welcome-back-subtitle = Elige una cuenta para seguir viendo.
login-welcome = Te damos la bienvenida a Tabbyfin
login-welcome-subtitle = Vamos a buscar tu servidor de Jellyfin.
login-address-required = Introduce la dirección de tu servidor.
login-username-required = Introduce tu nombre de usuario.
login-wrong-credentials = Nombre de usuario o contraseña incorrectos.
login-quick-connect-off = La Conexión rápida está desactivada en este servidor.
login-quick-connect-expired = El código de Conexión rápida ha caducado. Vuelve a intentarlo.
login-reconnect = Volverás a iniciar sesión automáticamente en cuanto el servidor responda.
login-remove-account = Quitar esta cuenta
login-server-address = Dirección del servidor
login-other-server = O conéctate a otro servidor
login-address-hint = jellyfin.example.com o 192.168.1.10:8096
login-connect = Conectar
login-username = Nombre de usuario
login-password = Contraseña
login-hide-password = Ocultar contraseña
login-show-password = Mostrar contraseña
login-sign-in = Iniciar sesión
login-quick-connect = Usar Conexión rápida
login-waiting-approval = Esperando la aprobación…
login-no-version = El servidor no ha indicado su versión.
login-version-too-old = Este servidor usa Jellyfin { $version }. Tabbyfin necesita la { $needed } o posterior.
login-server-starting = El servidor se está iniciando. Esperando…
login-not-jellyfin = Esa dirección no parece un servidor de Jellyfin.
login-error-certificate = El certificado del servidor no es de confianza. Comprueba la dirección o el certificado del servidor.
login-unencrypted-title = ¿Conectar sin cifrado?
login-unencrypted-notice = Este servidor no ha respondido por HTTPS, así que tu contraseña y lo que ves viajarían sin cifrar. Es habitual en un servidor de tu propia red y un riesgo en cualquier otro sitio.
login-connect-anyway = Conectar de todos modos
login-unreachable = No se ha podido contactar con el servidor.

dialog-sharing = Todos los de este servidor pueden verla
dialog-delete-note = La lista se elimina del servidor para todas las personas con las que se comparte. Lo que contiene se queda en tus bibliotecas.
dialog-added = Añadido a { $name }.
dialog-created = Se ha creado la lista { $name }.
dialog-already-there = { $item } ya está en { $name }.
dialog-cant-change = No puedes cambiar { $name }; es de otra persona.
dialog-add-to-playlist = Añadir a una lista
dialog-add-subtitle = Elige una de tus listas, o escribe un nombre para crear una nueva.
dialog-loading-playlists = Cargando listas…
dialog-no-playlist-named = No hay ninguna lista con ese nombre.
dialog-no-playlists = Aún no hay listas. Escribe un nombre para crear una.
dialog-edit-playlist = Editar lista
dialog-name = Nombre
dialog-checking-sharing = Comprobando quién puede verla…
dialog-save = Guardar
dialog-deleted = Se ha eliminado { $name }.
dialog-delete-forbidden = { $name } es de otra persona; no se puede eliminar.
dialog-delete-title = ¿Eliminar «{ $name }»?
dialog-delete = Eliminar
dialog-new-playlist = Lista nueva «{ $name }»
dialog-row-label = { $name }, { $detail }
dialog-item-count = { $count ->
    [one] { $digits } elemento
   *[other] { $digits } elementos
  }
dialog-playlist-forbidden = Esta lista es de otra persona; no se puede cambiar.

## downloads-and-casting

# Las descargas, y el aviso que la plataforma muestra de ellas (la notificación de Android).
downloads-title = Descargas
downloads-empty = Aún no hay nada descargado. Descarga una película o un episodio desde su página o su menú.
downloads-summary-done = { $count ->
    [one] { $count } descargado, { $size }
   *[other] { $count } descargados, { $size }
  }
downloads-summary-coming = { $count } por descargar
downloads-summary-failed = { $count ->
    [one] { $count } con error
   *[other] { $count } con error
  }
downloads-status-waiting-wifi = Esperando al wifi
downloads-status-waiting = Esperando para descargar
downloads-status-paused = En pausa
downloads-status-progress = { $done } de { $total } · { $percent } %
downloads-status-so-far = { $done } hasta ahora
downloads-status-failed = Error: { $message }
downloads-action-play = Reproducir
downloads-action-remove = Quitar descarga
downloads-action-retry = Reintentar descarga
downloads-action-remove-queued = Quitar de las descargas
downloads-action-cancel = Cancelar descarga
downloads-activity-waiting-wifi = Esperando al wifi
downloads-activity-downloading = Descargando
downloads-activity-to-come = { $count ->
    [one] { $count } descarga pendiente
   *[other] { $count } descargas pendientes
  }
downloads-activity-progress = { $done } de { $total }
downloads-activity-so-far = { $done } hasta ahora
downloads-activity-more = { $count } más por descargar
downloads-toast-downloaded = Se ha descargado { $title }
downloads-toast-failed = No se ha podido descargar { $title }: { $message }
downloads-error-not-downloadable = Este elemento no se puede descargar de este servidor.
downloads-error-not-media = Solo se pueden descargar vídeos, y las series y temporadas que los contienen.
downloads-error-stopped = la descarga se ha detenido inesperadamente
downloads-error-folder = no se ha podido crear su carpeta: { $error }
downloads-error-no-media = el servidor no ha devuelto ningún medio reproducible
downloads-error-write-item = no se ha podido escribir { $file }: { $error }
downloads-error-open = no se ha podido abrir el archivo: { $error }
downloads-error-restart = no se ha podido reiniciar el archivo: { $error }
downloads-error-continue = no se ha podido continuar el archivo: { $error }
downloads-error-write = no se ha podido escribir el archivo: { $error }
downloads-error-partial = solo han llegado { $done } de { $total } bytes
downloads-error-finish = no se ha podido terminar el archivo: { $error }
downloads-error-no-space = no hay suficiente espacio libre: hacen falta { $needed } y hay { $free } libres

# El envío a una TV: el panel, la barra y los avisos.
cast-title = Enviando
cast-status-casting = Enviando
cast-status-reconnecting = Reconectando
cast-panel-title = Enviar a una TV
cast-panel-note = Reproduce en un Chromecast o Google TV de esta red. Tu TV lo reproduce; esta aplicación es el mando.
cast-looking = Buscando TV en esta red…
cast-section-network = En esta red
cast-device-casting = { $device }, enviando
cast-device-reconnect = Reconectar con { $device }
cast-device-cast = Enviar a { $device }
cast-device-running = Jellyfin está abierto
cast-device-group = Grupo
cast-stop = Dejar de enviar
cast-disconnect-leave = Desconectar y dejar que siga reproduciendo
cast-back-10s = Retroceder 10 s
cast-forward-10s = Avanzar 10 s
cast-volume = Volumen de la TV
cast-stop-join-title = ¿Dejar de enviar para unirte?
cast-stop-create-title = ¿Dejar de enviar para crear un grupo?
cast-stop-note = Un grupo de SyncPlay se reproduce aquí, sincronizado con los demás, así que se deja de enviar a { $device }. Lo que reproduce también se detiene.
cast-bar-label-on = { $name }, en { $device }
cast-bar-label-reconnecting = { $name }, reconectando con { $device }
cast-bar-play = Reproducir
cast-bar-pause = Pausa
cast-bar-reconnecting = Reconectando con { $device }…
cast-bar-series = { $series } · en { $device }
cast-bar-on = en { $device }
cast-button = Enviar a una TV
cast-button-casting = Enviando (enviar a una TV)
cast-button-off-in-group = Enviar a una TV (desactivado en un grupo de SyncPlay)
cast-toast-casting = Enviando a { $device }
cast-toast-play-failed = La TV no ha podido reproducir esto ({ $code }).
cast-toast-unreachable = La TV no ha podido contactar con el servidor en { $address }. Necesita una dirección a la que la TV pueda llegar en esta red.
cast-toast-lost = Se ha perdido la conexión con { $device }.
cast-toast-stopped = { $device } ha dejado de recibir.
cast-toast-leave-group = Sal del grupo de SyncPlay para enviar a una TV.
cast-error-connect = No se ha podido conectar con { $device }.
cast-error-launch = { $device } no ha podido iniciar Jellyfin.
cast-error-wrong-app = { $device } ha iniciado una aplicación que no es el receptor de Jellyfin.
cast-unknown-error = error desconocido

# SyncPlay: las novedades del grupo, como avisos.
syncplay-joined = Te has unido al grupo de SyncPlay { $group }
syncplay-left = Has salido de { $group }
syncplay-gone = Ese grupo de SyncPlay ya no existe.
syncplay-no-access = Alguien del grupo no tiene acceso a este elemento.
syncplay-user-joined = { $user } se ha unido
syncplay-user-left = { $user } se ha ido
syncplay-error = SyncPlay: { $message }
syncplay-load-failed = No se ha podido cargar el elemento del grupo: { $message }
