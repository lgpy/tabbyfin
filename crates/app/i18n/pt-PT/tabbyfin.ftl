# A interface em português europeu: uma tradução de cada mensagem de `en/tabbyfin.ftl`, com
# as mesmas secções e argumentos.


## browse
home-title = Início
home-my-media = A minha multimédia
home-continue-watching = Continuar a ver
home-next-up = A seguir
home-favorites = Favoritos
home-latest = Mais recentes em { $library }
library-empty = O servidor ainda não tem bibliotecas.
library-title = Bibliotecas
library-count-movies = { $count ->
    [one] { $shown } filme
   *[other] { $shown } filmes
    }
library-count-shows = { $count ->
    [one] { $shown } série
   *[other] { $shown } séries
    }
library-count-collections = { $count ->
    [one] { $shown } coleção
   *[other] { $shown } coleções
    }
library-count-playlists = { $count ->
    [one] { $shown } lista de reprodução
   *[other] { $shown } listas de reprodução
    }
library-count-items = { $count ->
    [one] { $shown } item
   *[other] { $shown } itens
    }
library-sort-name = Nome
library-sort-added = Data de adição
library-sort-released = Data de lançamento
library-sort-rating = Classificação
library-sort-runtime = Duração
library-sort-random = Aleatório
library-progress-any = Qualquer estado
library-progress-unwatched = Por ver
library-progress-watched = Visto
library-progress-in-progress = A ver
library-resolution-any = Qualquer resolução
library-filter-sort = Ordenar por
library-filter-progress = Estado
library-filter-rating = Classificação
library-filter-year = Ano
library-filter-resolution = Resolução
library-filter-filters = Filtros
library-filter-clear = Limpar filtros
library-filter-values = Géneros e etiquetas
library-filter-order = Ordem
library-any-rating = Qualquer classificação
library-any-year = Qualquer ano
library-shuffle-again = Baralhar novamente
library-sort-ascending = Ordem ascendente
library-sort-descending = Ordem descendente
library-jump-to = Ir para { $letter }
library-nothing-matches = Nada corresponde a estes filtros.
library-nothing-yet = Ainda não há nada aqui.
library-item-count = { $count ->
    [one] { $count } item
   *[other] { $count } itens
    }
library-values-chosen-items = Escolhidos: { $chosen } · { $items }
library-values-chosen = Escolhidos: { $chosen }
library-values-none-match = Nenhum género ou etiqueta corresponde a “{ $search }”.
library-values-genres = Géneros
library-values-tags = Etiquetas
library-values-hint-touch = Toque uma vez para incluir, duas para excluir. Mantenha premido para limpar.
library-values-hint-mouse = Clique uma vez para incluir, duas para excluir. Clique com o botão direito para limpar.
library-values-find = Encontrar um género ou uma etiqueta
library-values-not = Exceto { $value }
library-values-count = { $count ->
    [one] { $count } filtro
   *[other] { $count } filtros
    }
library-values-two = { $first }, { $second }
library-done = Concluído
library-clear = Limpar
search-title = Pesquisar
search-type-to-search = Escreva na caixa de pesquisa para pesquisar.
search-no-results = Sem resultados.
search-movies = Filmes
search-shows = Séries
search-episodes = Episódios
item-seasons = Temporadas
item-in-this-collection = Nesta coleção
item-playlist = Lista de reprodução
item-episodes = Episódios
item-episode-count = { $count ->
    [one] { $count } episódio
   *[other] { $count } episódios
    }
item-to-watch = { $count } por ver
item-watched = Visto
item-no-episodes = Não há episódios nesta temporada.
item-chapters = Capítulos
item-cast-and-crew = Elenco e equipa
item-extras = Extras
item-more-like-this = Semelhantes
item-play = Reproduzir
item-play-episode = Reproduzir { $episode }
item-play-all = Reproduzir tudo
item-play-from-beginning = Reproduzir desde o início
item-resume = Retomar
item-resume-episode = Retomar { $episode }
item-resume-from = Retomar a partir de { $time }
item-shuffle = Aleatório
item-trailer = Trailer
item-more = Mais
item-favorite = Favorito
item-from-start = Desde o início
item-add-to-playlist = Adicionar à lista de reprodução…
item-edit = Editar…
item-delete-playlist = Eliminar lista de reprodução…
item-version = Versão
item-version-number = Versão { $number }
item-nothing-to-play = Não há nada para reproduzir.
item-download = Transferir
item-downloading = A transferir
item-downloaded = Transferido
item-download-all-episodes = Transferir todos os episódios
item-downloading-episodes = { $count ->
    [one] A transferir { $count } episódio
   *[other] A transferir { $count } episódios
    }
item-downloaded-rest = { $count ->
    [one] { $count } episódio transferido. Transferir os restantes
   *[other] { $count } episódios transferidos. Transferir os restantes
    }
item-waiting-to-download = A aguardar transferência. Cancelar transferência
item-waiting = A aguardar
item-downloading-progress = A transferir, { $progress }. Cancelar transferência
item-downloaded-size = Transferido, { $size }. Remover transferência
item-download-failed = A transferência falhou: { $message } Tentar transferir novamente
item-retry = Repetir
item-cancel-download = Cancelar transferência
item-remove-download = Remover transferência
item-retry-download = Tentar transferir novamente
item-left = Faltam { $time }
item-less = Menos
item-show-less = Mostrar menos
item-show-more = Mostrar mais
item-director = Realização
item-writers = Argumento
item-studio = Estúdio
item-playlist-not-yours = Esta lista de reprodução pertence a outra pessoa e não pode ser alterada.
widget-back = Voltar
widget-cancel = Cancelar
widget-retry = Tentar novamente
widget-scroll-left = Deslocar para a esquerda
widget-scroll-right = Deslocar para a direita
widget-move-up = Mover para cima (Alt+Cima)
widget-move-down = Mover para baixo (Alt+Baixo)
widget-remove-from-playlist = Remover da lista de reprodução (Delete)
widget-watched-key = Visto (W)
widget-copy-code = Copiar código
widget-copied = Copiado
widget-error = Erro
widget-information = Informação
widget-success = Sucesso
widget-decrease = Diminuir { $label }
widget-increase = Aumentar { $label }
widget-reset = Repor { $label }
widget-chapter-number = Capítulo { $number }
widget-unwatched = { $count } por ver
widget-included = { $label }, incluído
widget-excluded = { $label }, excluído

## player
player-back = Voltar (Esc)
player-starting = A iniciar a reprodução…
player-play = Reproduzir (Espaço)
player-pause = Pausar (Espaço)
player-back-10s = Recuar 10 s (Esquerda)
player-forward-10s = Avançar 10 s (Direita)
player-mute = Desativar o som (M)
player-unmute = Ativar o som (M)
player-fullscreen = Ecrã inteiro (F)
player-exit-fullscreen = Sair do ecrã inteiro (F)
player-settings = Definições de reprodução
player-subtitles = Legendas
player-audio = Áudio
player-chapters = Capítulos
player-seek = Posição
player-volume = Volume
player-cast = Transmitir para uma TV
player-cast-to-device = A transmitir para { $device }
player-playing-on = A reproduzir em { $device }
player-reconnecting-to = A restabelecer a ligação a { $device }…
player-resumed-from = Retomado a partir de { $time }
player-start-over = Começar de novo
player-up-next = A seguir
player-play-now = Reproduzir agora
player-cancel = Cancelar
player-chapter-number = Capítulo { $number }
player-previous = Anterior (Shift+P)
player-previous-episode = Episódio anterior (Shift+P)
player-previous-movie = Filme anterior (Shift+P)
player-next = Seguinte (Shift+N)
player-next-episode = Episódio seguinte (Shift+N)
player-next-movie = Filme seguinte (Shift+N)
player-version = Versão
player-version-number = Versão { $number }
player-tab-general = Geral
player-no-audio-tracks = Sem faixas de áudio
player-no-subtitles = Sem legendas
player-speed = Velocidade
player-size = Tamanho
player-delay = Atraso
player-normal-in-syncplay = Normal em SyncPlay
player-badge-syncplay = SyncPlay
player-badge-transcoding = Transcodificação
player-sheet-settings = Definições
player-close = Fechar
player-subtitles-off = Desligadas
player-track-number = Faixa { $index }
player-skip-intro = Saltar genérico
player-skip-recap = Saltar resumo
player-skip-preview = Saltar antevisão
player-skip-ad = Saltar anúncio
player-skip-credits = Saltar créditos
player-restarting = O leitor parou inesperadamente. A reiniciá-lo…
player-reconnecting = Perdeu-se a ligação ao servidor. A restabelecer a ligação…
player-crashing = O leitor está sempre a falhar neste item. O registo pode explicar porquê.
player-connection-lost = Perdeu-se a ligação ao servidor. Tente novamente quando o servidor voltar.
player-not-streamable = O servidor não consegue transmitir este item para esta aplicação.
player-server-refused = O servidor não vai reproduzir este item ({ $code }).
player-no-media = O servidor não devolveu nada que se possa reproduzir.
player-start-failed = Não foi possível iniciar o leitor: { $error }
player-mpv-android = O mpv não conseguiu arrancar. Escolha o Media3 como leitor em Definições › Reprodução.
player-mpv-missing-macos = A reprodução de vídeo precisa do libmpv, que não foi encontrado. Instale o mpv com o Homebrew (brew install mpv).
player-mpv-missing-windows = A reprodução de vídeo precisa do libmpv, que não foi encontrado. Falta o libmpv-2.dll junto do tabbyfin.exe; reinstale a aplicação.
player-mpv-missing-linux = A reprodução de vídeo precisa do libmpv, que não foi encontrado. Instale o libmpv (o pacote libmpv2 no Debian e no Ubuntu).
player-download-path = O caminho do ficheiro transferido não é válido: { $path }
player-stream-open-failed = O leitor não conseguiu abrir o fluxo: { $error }
player-play-failed = O leitor não conseguiu reproduzir este ficheiro: { $error }

## settings
settings-title = Definições
settings-store-device = Este dispositivo
settings-store-server = Servidor Jellyfin
settings-section-appearance = Aspeto
settings-section-appearance-summary = Tema, tamanho do texto, animações e idioma
settings-section-playback = Reprodução
settings-section-playback-summary = Episódio seguinte, genéricos, qualidade e o leitor
settings-section-subtitles = Legendas
settings-section-subtitles-summary = Cor, estilo e tamanho
settings-section-downloads = Transferências
settings-section-downloads-summary = Apenas por Wi-Fi, e onde ficam guardadas
settings-section-storage = Armazenamento
settings-section-storage-summary = Cache de imagens e registos
settings-section-about = Acerca de
settings-section-about-summary = Versão e atualizações
settings-section-account = Conta
settings-section-account-summary = Quem tem sessão iniciada e o servidor
settings-section-languages = Idiomas
settings-section-languages-summary = Faixas de áudio e de legendas
settings-section-home = Início
settings-section-home-summary = O que as linhas de Início mostram
settings-language = Idioma
settings-language-description = O idioma da aplicação. Aplica-se de imediato.
settings-language-unset = o idioma do sistema
settings-language-system = Seguir o sistema ({ $language })
settings-theme = Tema
settings-theme-description = Escuro, claro ou o esquema de cores do ambiente de trabalho. Aplica-se de imediato.
settings-theme-unset = escuro
settings-theme-system = Seguir o sistema
settings-theme-light = Claro
settings-theme-dark = Escuro
settings-text-size = Tamanho do texto
settings-text-size-description = Aumenta ou reduz o texto e os controlos da aplicação, não a disposição. Aplica-se de imediato.
settings-text-size-unset = o tamanho de texto do sistema
settings-text-size-system = Seguir o sistema ({ $scale }%)
settings-animations = Animações
settings-animations-description = Animações e deslocamento suave. Aplica-se de imediato.
settings-animations-unset = ativadas
settings-tracks-note = As faixas de áudio e de legendas com que um item começa ficam guardadas no servidor, para todas as aplicações que utilizar.
settings-autoplay = Reproduzir o episódio seguinte automaticamente
settings-autoplay-description = Quando um episódio termina, para o leitor ficar aberto. A partir do item seguinte.
settings-autoplay-unset = ativado
settings-skip-intro = Saltar genéricos e resumos automaticamente
settings-skip-intro-description = Cada genérico ou resumo uma vez, ao começar, nunca num grupo SyncPlay. A partir do item seguinte.
settings-skip-intro-unset = desativado
settings-bitrate = Taxa de bits máxima de transmissão
settings-bitrate-description = Os vídeos acima desta taxa de bits são transcodificados pelo servidor. Reduza-a para ligações lentas. Automática mede a ligação antes de reproduzir (no máximo a cada 10 minutos) e usa 80% dela, mantendo o limite para quando for desativada. Aplica-se quando o item seguinte começar.
settings-bitrate-unset = sem limite
settings-bitrate-auto-measured = Automática (medida)
settings-bitrate-auto = Automática
settings-bitrate-unlimited = Sem limite
settings-cast-bitrate = Taxa de bits do Chromecast
settings-cast-bitrate-description = A taxa máxima a que um Chromecast ou uma Google TV reproduz quando recebe uma transmissão; o servidor transcodifica tudo o que estiver acima. Automática deixa a decisão à TV, que mede a sua ligação. É enviada com o comando seguinte para a TV, que a mantém até ser alterada.
settings-cast-bitrate-unset = automática
settings-player = Leitor
settings-player-description = O Media3 reproduz com os descodificadores do próprio Android, mantém o HDR e gasta menos bateria. O mpv reproduz mais tipos de ficheiros e mostra as legendas com estilo (ASS) como foram feitas. Aplica-se quando o item seguinte começar.
settings-player-unset = Media3
settings-hevc = Pedir HEVC quando o servidor transcodificar
settings-hevc-description = O HEVC precisa de metade da largura de banda do H.264 para a mesma imagem, mas é muito mais difícil de codificar pelo servidor. Usado apenas se o servidor o permitir (Painel › Reprodução › Transcodificação). Aplica-se quando o item seguinte começar.
settings-hevc-unset = desativado
settings-subtitles-note = O aspeto das legendas de texto como as SRT; as legendas com estilo (ASS) mantêm o seu aspeto e só mudam de tamanho. O menu de definições do leitor também muda o tamanho durante a reprodução.
settings-subtitle-tracks-note = As legendas que se ativam sozinhas, e em que idioma, ficam guardadas no servidor.
settings-subtitle-color = Cor
settings-subtitle-color-description = Das legendas de texto. Aplica-se quando o item seguinte começar.
settings-subtitle-color-unset = branco
settings-color-white = Branco
settings-color-yellow = Amarelo
settings-subtitle-style = Estilo
settings-subtitle-style-description = Um contorno, ou uma caixa escura translúcida, atrás das legendas de texto. Aplica-se quando o item seguinte começar.
settings-subtitle-style-unset = contorno
settings-style-outline = Contorno
settings-style-box = Caixa escura
settings-subtitle-size = Tamanho
settings-subtitle-size-description = Partilhado com o menu de definições do leitor. Chega de imediato ao gestor de reprodução, por isso uma sessão em curso muda com ele.
settings-subtitle-size-unset = 100 %
settings-wifi-only = Transferir apenas por Wi-Fi
settings-wifi-only-description = Em dados móveis, as transferências esperam pelo Wi-Fi, e uma em curso para e continua de onde estava assim que o Wi-Fi volta. Aplica-se de imediato.
settings-wifi-only-unset = ativado
settings-downloads-folder = Pasta das transferências
settings-downloads-folder-description = A pasta da aplicação no armazenamento partilhado do telemóvel. Desinstalar a aplicação elimina-a.
settings-image-cache = Cache de imagens
settings-image-cache-description = Os cartazes e os fundos ficam guardados aqui e voltam a carregar quando as páginas os pedem outra vez.
settings-logs = Registos
settings-logs-description = É escrito um ficheiro de registo por dia. Anexe-o aos relatórios de erros.
settings-version = Versão
settings-check-for-updates = Avisar-me quando sair uma nova versão
settings-check-for-updates-description = Verifica as versões do GitHub uma vez por dia, quando a aplicação arranca.
settings-check-for-updates-unset = ativado
settings-user = Utilizador
settings-server = Servidor
settings-address = Endereço
settings-server-version = Versão do Jellyfin
settings-languages-note = O servidor escolhe entre estas as faixas com que um item começa. Os menus do leitor mudam-nas durante a reprodução. A partir do item seguinte.
settings-audio-language = Idioma do áudio
settings-audio-language-description = A faixa de áudio neste idioma, quando o ficheiro a tem.
settings-play-default-audio = Reproduzir a faixa de áudio predefinida
settings-play-default-audio-description = A faixa que o ficheiro marca como predefinida, seja qual for o idioma.
settings-remember-audio = Memorizar a faixa de áudio que escolho
settings-remember-audio-description = Uma faixa escolhida manualmente é escolhida outra vez para os episódios seguintes da série.
settings-subtitle-language = Idioma das legendas
settings-subtitle-language-description = O idioma que o modo de legendas procura.
settings-subtitle-mode = Modo de legendas
settings-subtitle-mode-description = Que legendas se ativam sozinhas. Predefinido: as que o ficheiro marca como predefinidas ou forçadas. Inteligente: as do idioma das legendas quando o áudio é noutro. Apenas forçadas: só as forçadas. Sempre: as do idioma das legendas, seja qual for o áudio. Nenhum: nenhumas até se escolher uma.
settings-mode-default = Predefinido
settings-mode-smart = Inteligente
settings-mode-only-forced = Apenas forçadas
settings-mode-always = Sempre
settings-mode-none = Nenhum
settings-remember-subtitles = Memorizar as legendas que escolho
settings-remember-subtitles-description = As legendas escolhidas manualmente, ou desativadas, são escolhidas outra vez para os episódios seguintes da série.
settings-hide-played-in-latest = Deixar de fora os itens vistos em Recentes
settings-hide-played-in-latest-description = As linhas Recentes de Início mostram apenas o que ainda não viu. Início volta a carregar ao guardar.
settings-home-sections-note = As secções que Início mostra, e as bibliotecas nelas, seguem o seu perfil Jellyfin (Definições › Início no cliente web).
settings-no-match = Nenhuma definição corresponde a “{ $query }”.
settings-kept-on-device = Guardadas neste computador, para todas as contas aqui com sessão iniciada.
settings-kept-in-account = Guardadas na conta de { $user } no servidor, para que o cliente web e outras aplicações Jellyfin as partilhem.
settings-kept-in-account-on = Guardadas na conta de { $user } em { $server }, para que o cliente web e outras aplicações Jellyfin as partilhem.
settings-saving = A guardar…
settings-saved-to-server = Guardadas no servidor
settings-signed-in-as = Sessão iniciada como { $user } em { $server }
settings-key-config = { $key } no ficheiro de configuração
settings-key-config-unset = { $key } no ficheiro de configuração · não definido: { $unset }
settings-key-profile = { $key } no seu perfil Jellyfin
settings-reset-default = Repor o valor predefinido
settings-reset-default-tooltip = Repor o valor predefinido: { $default }
settings-on = Ativado
settings-off = Desativado
settings-clear = Limpar
settings-open-folder = Abrir pasta
settings-image-cache-cleared = Cache de imagens limpa.
settings-image-cache-failed = Não foi possível limpar a cache de imagens. Feche o que estiver a usar a pasta e tente novamente.
settings-logs-folder-failed = Não foi possível abrir a pasta dos registos ({ $path }).
settings-save-failed = Não foi possível guardar as suas definições no servidor. Voltaram a estar como o servidor as tem.
settings-load-failed = Não foi possível carregar as suas definições do servidor.
settings-loading = A carregar as suas definições do servidor…
settings-retry = Tentar novamente
settings-any-language = Qualquer idioma

## shell

shell-home = Início
shell-libraries = Bibliotecas
shell-back = Voltar
shell-settings = Definições
shell-syncplay = Ver em conjunto (SyncPlay)
shell-search = Pesquisar
shell-search-page = Pesquisar { $page }
shell-clear-search = Limpar pesquisa
menu-filter = Filtrar
menu-no-matches = Nada corresponde a “{ $search }”.
shell-account-on = { $name } em { $server }
shell-downloads = Transferências
shell-add-account = Adicionar conta…
shell-sign-out = Terminar sessão

shell-syncplay-title = Ver em conjunto
shell-syncplay-description = Reproduza em sincronia com outras pessoas neste servidor.
shell-syncplay-empty = Não há ninguém a ver em conjunto neste momento.
shell-syncplay-join-section = Juntar-se a um grupo
shell-syncplay-new-group = Novo grupo
shell-syncplay-anything = Tudo o que reproduzir agora é reproduzido para todos aqui.
shell-syncplay-leave = Sair do grupo
shell-syncplay-join-label = Juntar-se a { $group }, com { $people }
shell-syncplay-join = Juntar-se
shell-people-nobody = ainda ninguém
shell-people-and = { $others } e { $last }
shell-group-status-playing = A reproduzir
shell-group-status-waiting = A aguardar por todos
shell-group-status-paused = Em pausa
shell-group-status-idle = Nada a reproduzir
shell-group-subtitle = { $people } · { $status }
shell-group-watching = A ver · { $count }
shell-group-you = eu
shell-queue = Fila
shell-queue-repeat-off = Repetição desativada
shell-queue-repeat-all = A repetir a fila
shell-queue-repeat-one = A repetir um
shell-queue-shuffle-on = Aleatório ativado
shell-queue-shuffle-off = Aleatório desativado
shell-queue-playing = { $name }, a reproduzir
shell-queue-remove = Remover (Delete)
shell-new-group-name = Grupo de { $user }

shell-offline = Sem ligação
shell-offline-hint = Não é possível contactar o servidor. As suas transferências estão aqui e a aplicação continua a tentar.

shell-update-message = Já saiu a versão { $version } (esta é a { $current }).
shell-update-title = Atualização disponível
shell-update-download = Transferir
shell-back-online = A ligação ao servidor voltou.

shell-toast-close = Fechar
shell-toast-information = Informação
shell-toast-success = Sucesso
shell-toast-error = Erro

shell-error-invalid-url = Esse não parece ser um endereço de servidor válido.
shell-error-timeout = O servidor demorou demasiado tempo a responder.
shell-error-connect = Não foi possível contactar o servidor. Verifique o endereço e a ligação.
shell-error-broken = A ligação ao servidor foi interrompida. Pode estar a reiniciar.
shell-error-starting = O servidor ainda está a arrancar.
shell-error-expired = A sua sessão expirou. Inicie sessão novamente.
shell-error-forbidden = Esta conta não tem permissão para isso.
shell-error-not-found = O servidor não encontrou o que foi pedido.
shell-error-decode = O servidor enviou uma resposta que esta aplicação não compreende.
shell-error-stalled = O servidor não enviou nada durante 30 segundos.
shell-error-socket = A ligação em tempo real ao servidor falhou.
shell-error-status = O servidor devolveu um erro ({ $status }).

login-cancel = Cancelar
login-use-another-server = Usar outro servidor
login-min-version = Funciona com o Jellyfin { $version } ou mais recente
login-back-to-account = Voltar à sua conta
login-quick-connect-title = Aprovar este início de sessão
login-quick-connect-subtitle = Introduza este código no Quick Connect de uma aplicação Jellyfin em que tenha sessão iniciada.
login-to-server = Iniciar sessão em { $server }
login-server-line = Jellyfin { $version } em { $host }
login-welcome-back = Olá de novo
login-welcome-back-subtitle = Escolha uma conta para continuar a ver.
login-welcome = Boas-vindas ao Tabbyfin
login-welcome-subtitle = Vamos encontrar o seu servidor Jellyfin.
login-address-required = Introduza o endereço do servidor.
login-username-required = Introduza o nome de utilizador.
login-wrong-credentials = Nome de utilizador ou palavra-passe incorretos.
login-quick-connect-off = O Quick Connect está desativado neste servidor.
login-quick-connect-expired = O código do Quick Connect expirou. Tente novamente.
login-reconnect = A sessão será retomada automaticamente assim que o servidor responder.
login-remove-account = Remover esta conta
login-server-address = Endereço do servidor
login-other-server = Ou ligue-se a outro servidor
login-address-hint = jellyfin.example.com ou 192.168.1.10:8096
login-connect = Ligar
login-username = Nome de utilizador
login-password = Palavra-passe
login-hide-password = Ocultar palavra-passe
login-show-password = Mostrar palavra-passe
login-sign-in = Iniciar sessão
login-quick-connect = Usar Quick Connect
login-waiting-approval = A aguardar aprovação…
login-no-version = O servidor não indicou a versão.
login-version-too-old = Este servidor tem o Jellyfin { $version }. O Tabbyfin precisa da versão { $needed } ou mais recente.
login-server-starting = O servidor está a arrancar. A aguardar…
login-not-jellyfin = Esse endereço não parece ser um servidor Jellyfin.
login-unreachable = Não foi possível contactar o servidor.

dialog-sharing = Todos neste servidor podem vê-la
dialog-delete-note = A lista de reprodução é removida do servidor para todos com quem está partilhada. O que está nela fica nas suas bibliotecas.
dialog-added = Adicionado a { $name }.
dialog-created = Lista de reprodução { $name } criada.
dialog-already-there = { $item } já está em { $name }.
dialog-cant-change = Não pode alterar { $name }; pertence a outra pessoa.
dialog-add-to-playlist = Adicionar à lista de reprodução
dialog-add-subtitle = Escolha uma das suas listas de reprodução ou escreva um nome para criar uma nova.
dialog-loading-playlists = A carregar listas de reprodução…
dialog-no-playlist-named = Nenhuma lista de reprodução tem esse nome.
dialog-no-playlists = Ainda não tem listas de reprodução. Escreva um nome para criar uma.
dialog-edit-playlist = Editar lista de reprodução
dialog-name = Nome
dialog-checking-sharing = A verificar quem a pode ver…
dialog-save = Guardar
dialog-deleted = { $name } eliminada.
dialog-delete-forbidden = { $name } pertence a outra pessoa e não pode ser eliminada.
dialog-delete-title = Eliminar “{ $name }”?
dialog-delete = Eliminar
dialog-new-playlist = Nova lista de reprodução “{ $name }”
dialog-row-label = { $name }, { $detail }
dialog-item-count = { $count ->
    [one] { $digits } item
   *[other] { $digits } itens
  }
dialog-playlist-forbidden = Esta lista de reprodução pertence a outra pessoa; não pode ser alterada.

## downloads-and-casting

# As transferências, e o aviso que a plataforma mostra delas (a notificação do Android).
downloads-title = Transferências
downloads-empty = Ainda não há nada transferido. Transfira um filme ou um episódio a partir da respetiva página ou do menu.
downloads-summary-done = { $count ->
    [one] { $count } transferido, { $size }
   *[other] { $count } transferidos, { $size }
  }
downloads-summary-coming = { $count } por transferir
downloads-summary-failed = { $count ->
    [one] { $count } falhou
   *[other] { $count } falharam
  }
downloads-status-waiting-wifi = À espera de Wi-Fi
downloads-status-waiting = Em fila
downloads-status-paused = Em pausa
downloads-status-progress = { $done } de { $total } · { $percent }%
downloads-status-so-far = { $done } até agora
downloads-status-failed = Falhou: { $message }
downloads-action-play = Reproduzir
downloads-action-remove = Remover transferência
downloads-action-retry = Tentar novamente
downloads-action-remove-queued = Remover das transferências
downloads-action-cancel = Cancelar transferência
downloads-activity-waiting-wifi = À espera de Wi-Fi
downloads-activity-downloading = A transferir
downloads-activity-to-come = { $count ->
    [one] Falta { $count } transferência
   *[other] Faltam { $count } transferências
  }
downloads-activity-progress = { $done } de { $total }
downloads-activity-so-far = { $done } até agora
downloads-activity-more = mais { $count } por transferir
downloads-toast-downloaded = Transferido: { $title }
downloads-toast-failed = Não foi possível transferir { $title }: { $message }
downloads-error-not-downloadable = Este item não pode ser transferido a partir deste servidor.
downloads-error-not-media = Só é possível transferir vídeos e as séries e temporadas que os contêm.
downloads-error-stopped = a transferência parou inesperadamente
downloads-error-folder = não foi possível criar a pasta: { $error }
downloads-error-no-media = o servidor não devolveu nenhum ficheiro reproduzível
downloads-error-write-item = não foi possível escrever { $file }: { $error }
downloads-error-open = não foi possível abrir o ficheiro: { $error }
downloads-error-restart = não foi possível recomeçar o ficheiro: { $error }
downloads-error-continue = não foi possível continuar o ficheiro: { $error }
downloads-error-write = não foi possível escrever o ficheiro: { $error }
downloads-error-partial = só chegaram { $done } de { $total } bytes
downloads-error-finish = não foi possível concluir o ficheiro: { $error }
downloads-error-no-space = não há espaço livre suficiente: são necessários { $needed } e há { $free } livres

# A transmissão: o painel, a barra e os avisos.
cast-title = Transmissão
cast-status-casting = A transmitir
cast-status-reconnecting = A religar
cast-panel-title = Transmitir para uma TV
cast-panel-note = Reproduza num Chromecast ou Google TV nesta rede. É a TV que reproduz; esta aplicação serve de comando.
cast-looking = A procurar TVs nesta rede…
cast-section-network = Nesta rede
cast-device-casting = { $device }, a transmitir
cast-device-reconnect = Voltar a ligar a { $device }
cast-device-cast = Transmitir para { $device }
cast-device-running = O Jellyfin está a funcionar
cast-device-group = Grupo
cast-stop = Parar a transmissão
cast-disconnect-leave = Desligar e deixar a reproduzir
cast-back-10s = Recuar 10 s
cast-forward-10s = Avançar 10 s
cast-volume = Volume da TV
cast-stop-join-title = Parar a transmissão para se juntar?
cast-stop-create-title = Parar a transmissão para criar um grupo?
cast-stop-note = Um grupo SyncPlay reproduz aqui, em sintonia com os outros, por isso a transmissão para { $device } termina. O que está a reproduzir também termina.
cast-bar-label-on = { $name }, em { $device }
cast-bar-label-reconnecting = { $name }, a restabelecer a ligação a { $device }
cast-bar-play = Reproduzir
cast-bar-pause = Pausar
cast-bar-reconnecting = A restabelecer a ligação a { $device }…
cast-bar-series = { $series } · em { $device }
cast-bar-on = em { $device }
cast-button = Transmitir para uma TV
cast-button-casting = A transmitir (transmitir para uma TV)
cast-button-off-in-group = Transmitir para uma TV (indisponível num grupo SyncPlay)
cast-toast-casting = A transmitir para { $device }
cast-toast-play-failed = A TV não conseguiu reproduzir isto ({ $code }).
cast-toast-unreachable = A TV não conseguiu chegar ao servidor em { $address }. É preciso um endereço a que a TV consiga chegar nesta rede.
cast-toast-lost = Perdeu-se a ligação a { $device }.
cast-toast-stopped = { $device } deixou de transmitir.
cast-toast-leave-group = Saia do grupo SyncPlay para transmitir para uma TV.
cast-error-connect = Não foi possível ligar a { $device }.
cast-error-launch = { $device } não conseguiu iniciar o Jellyfin.
cast-error-wrong-app = { $device } iniciou uma aplicação que não é o recetor do Jellyfin.
cast-unknown-error = erro desconhecido

# O SyncPlay: as atualizações do grupo, como avisos.
syncplay-joined = Juntou-se ao grupo SyncPlay { $group }
syncplay-left = Saiu de { $group }
syncplay-gone = Esse grupo SyncPlay já não existe.
syncplay-no-access = Alguém no grupo não tem acesso a este item.
syncplay-user-joined = { $user } juntou-se ao grupo
syncplay-user-left = { $user } saiu do grupo
syncplay-error = SyncPlay: { $message }
syncplay-load-failed = Não foi possível carregar o item do grupo: { $message }

item-search-subtitles = Procurar legendas…
player-search-subtitles = Procurar legendas…
subtitle-search-title = Procurar legendas
subtitle-search-language = Idioma
subtitle-search-searching = A procurar…
subtitle-search-none = Não foram encontradas legendas em { $language }. Se uma pesquisa nunca encontrar nenhuma, o servidor pode não ter um fornecedor de legendas configurado; um administrador pode adicionar um, como o Open Subtitles, nos plugins do painel de controlo.
subtitle-search-forbidden = A sua conta não tem permissão para procurar legendas.
subtitle-search-downloading = A transferir…
subtitle-search-added = Legenda adicionada
subtitle-search-not-downloaded = O servidor não conseguiu transferir essa legenda. Experimente outra.
subtitle-search-exact = Correspondência exata
subtitle-search-downloads = { $count ->
    [one] { $digits } transferência
   *[other] { $digits } transferências
  }
subtitle-search-rating = Classificação { $rating }
subtitle-search-row-label = { $name }, { $detail }
player-position = Posição
settings-subtitle-position = Posição
settings-subtitle-position-description = A que altura do ecrã ficam as legendas. Partilhada com o menu de definições do leitor. Chega de imediato ao gestor de reprodução, por isso uma sessão em curso muda com ela.
settings-subtitle-position-unset = Em baixo
settings-subtitle-position-bottom = Em baixo
settings-subtitle-position-raised = { $percent } % mais acima
