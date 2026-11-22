# Heistarr: plan del proyecto (v0.4, revisado)

> Heistarr (nombre elegido; libre en GitHub, crates.io, Docker Hub y npm). Gestor multimedia autoalojado en Rust, inspirado en
> HolyArrs (que sustituye a Sonarr, Radarr, Lidarr, Prowlarr, Bazarr y Overseerr), escrito desde
> cero: sin código, nombre ni marca de HolyArrs (que es AGPL-3.0). Repositorio privado y sin
> licencia pública por ahora.

## 1. Objetivo

Una sola aplicación, un solo login y un solo puerto para series, películas y música:
explorar un catálogo, pedir títulos, buscar en indexadores, elegir el mejor release con reglas,
mandarlo al cliente de descargas, importar el archivo a la biblioteca con un nombre limpio,
avisar a Plex/Jellyfin/Emby y bajar subtítulos.

**Fuera de alcance (igual que HolyArrs):**
- No descarga por sí mismo: usa qBittorrent, SABnzbd, etc.
- No reproduce el contenido: eso lo hacen Plex/Jellyfin/Emby.
- Nada de apps nativas para móvil/TV; la web funciona en el móvil.
- No usamos un servidor de metadatos propio: cada instalación usa su propia clave de TMDB
  (y MusicBrainz para música, que no necesita clave).

## 2. Arquitectura

```
heistarr/
  crates/
    core/      tipos compartidos, parser de nombres de release, reglas de selección (sin E/S)
    web/       la web en Leptos (Rust a wasm, en el navegador): componentes y páginas
    server/    binario `heistarr`: axum, API JSON interna (`/api`), SQLite,
               tareas en segundo plano; sirve también la web ya compilada
    desktop/   (fase 9) Tauri envolviendo la misma web
  style/       CSS: base + un archivo de variables por tema
  docs/
```

- **Un solo binario** más la carpeta de estáticos (wasm, CSS), todo en la imagen Docker
  (amd64 y arm64, para el NAS). Puerto por defecto: **6464**.
- **Interfaz**: Leptos (Rust a wasm) que se ejecuta en el navegador y habla con el servidor por
  una API JSON interna; se compila con `scripts/build-web.sh` (wasm-bindgen). Sin renderizado en
  servidor por ahora: más simple de construir en Docker, y la app es de uso privado.
  HTML/CSS real: vídeo HTML5 y tráileres de YouTube, seleccionar texto, Ctrl+F, accesibilidad.
- **Temas**: variables CSS. Por defecto **Miami Neón** (índigo #0c0620, rosa #ff3ea5,
  cian #2ee8ff, rótulos con brillo, sol de atardecer y rejilla). Alternativas en Ajustes:
  Chunkscope, Neon Noir, Hazard, Synthwave, Terminal (los de los mockups).
- **Reglas que mantenemos de Chunkscope**: todo campo acepta pegar (Ctrl+V) y todo valor
  mostrado se puede copiar.
- **Idioma**: interfaz, código y documentación en **inglés**.
- **Datos**: SQLite en la carpeta de datos (`/config` en Docker). Migraciones versionadas.
- **Red**: todas las llamadas salientes pasan por un único cliente HTTP (tiempos de espera,
  reintentos, registro) para poder auditarlas.
- **Tareas**: un planificador interno (buscar lo que falta, sincronizar RSS, revisar
  descargas, importar, limpiar historial).
- **Seguridad**: sin contraseña por defecto; el primer visitante crea la cuenta del dueño.
  Contraseñas con argon2, cookie de sesión HttpOnly, límite de intentos de login y protección
  CSRF. Secretos (claves de API, contraseñas de qBittorrent) **cifrados desde la fase 1** con
  una clave maestra; nunca se devuelven completos a la interfaz (solo "••••1234").
- **API**: la interfaz usa una API JSON interna (`/api`, sin versión ni promesas de estabilidad).
  Una API REST pública y versionada (`/api/v1`, con clave de API) llega en la fase 7.
- **Docker en el NAS**: `PUID`/`PGID`/`UMASK` para que los archivos creados tengan el dueño
  correcto, y **un solo volumen `/data`** con `torrents/` y `media/` dentro, para que los
  hardlinks funcionen (si descargas y biblioteca están en volúmenes distintos, no se puede
  hacer hardlink y se copiaría el archivo, duplicando el espacio).

## 3. Fases

Tamaño relativo: S (días), M (1-2 semanas), L (varias semanas). Cada fase termina con
tests en verde, una versión etiquetada y notas de versión.

### Fase 0: Fundaciones (M)
- Workspace con cargo-leptos, CI (fmt, clippy, tests, build wasm, Playwright), Dockerfile.
- Servidor axum + Leptos + SQLite + migraciones.
- Tema Miami Neón (y el selector de temas), navegación (barra lateral), pantallas vacías.
- Cuentas: asistente de primer arranque (crear dueño), login, logout.
- Ajustes guardados en la base de datos; página de ajustes básica.
- Registros (logs) a archivo con rotación y comprobación de salud (`/healthz`) para Docker.
- **Hecho cuando**: `docker run` arranca, creas el dueño en el navegador y ves la app vacía
  con el tema Miami Neón.

### Fase 1: Películas, de principio a fin (L) ← equivalente a Radarr
- Catálogo TMDB: buscar películas, ficha con póster y sinopsis (pósters a través del servidor,
  con caché).
- Biblioteca: añadir, quitar, estados (buscada / descargando / disponible).
- **Importar la biblioteca que ya tienes**: escanear la carpeta de películas, reconocer cada
  archivo contra TMDB y marcarlo como disponible (sin esto, la app intentaría bajar lo que ya
  tienes).
- Idioma de los metadatos de TMDB configurable (títulos y sinopsis en español aunque la
  interfaz esté en inglés).
- Indexadores Torznab (Jackett/Prowlarr), con botón de probar.
- Parser de nombres de release y reglas de selección (resolución mín/máx, tamaño, palabras
  preferidas/bloqueadas, semillas, códec) **e idioma del audio** (SPANISH, CASTELLANO, LATINO,
  MULTi, DUAL…), clave para contenido en español.
- Mejoras automáticas: si sale un release mejor que el que tienes, sustituirlo hasta llegar a
  un "corte" de calidad.
- Descargas fallidas o atascadas: lista de releases bloqueados y reintento con el siguiente.
- Búsqueda interactiva (ver todos los releases con su puntuación y por qué se rechazan) y
  automática.
- Clientes de descarga: **qBittorrent** y **Synology Download Station** (DSM 6 y 7; el caso de
  uso principal): enviar, seguir progreso, borrar o seguir sembrando según ajuste, con
  traducción de rutas (la ruta que ve el cliente → la que ve Heistarr).
- Importador: localizar el vídeo, hardlink (o copia si no se puede) a
  `Películas/Título (Año)/Título (Año).ext`.
- Avisar a Jellyfin/Plex para que reescaneen.
- Aviso de atribución de TMDB (obligatorio en sus condiciones de uso) en la pantalla
  "About".
- Páginas de Descargas e Historial.
- **Hecho cuando**: añades una película y acaba sola en la biblioteca de Jellyfin.
  Probado con servidores falsos de Torznab y qBittorrent en los tests de integración.

### Fase 2: Series (L) ← equivalente a Sonarr
- Series, temporadas y episodios desde TMDB; qué episodios seguir.
- Parser `S01E02`, packs de temporada, episodios dobles, especiales.
- Sincronización RSS de los indexadores para pillar episodios nuevos.
- Calendario de próximos episodios.
- Renombrado: `Series/Nombre (Año)/Season 01/Nombre - S01E02 - Título.ext`.

### Fase 3: Explorar y peticiones (L) ← equivalente a Overseerr
- Pantalla de inicio tipo Netflix: tendencias, populares, próximos estrenos, por género.
- Ficha del título: reparto, puntuaciones, tráiler (vídeo de YouTube incrustado o HTML5), un botón para pedir.
- Varios usuarios con roles: dueño, de confianza (se aprueba solo), normal (va a la cola).
- Cola de aprobación; pedir una serie entera o una temporada.
- Listas de seguimiento de Plex/Jellyfin convertidas en peticiones.

### Fase 4: Más clientes e indexadores (M) ← equivalente a Prowlarr
- Indexadores Newznab (usenet); estado/salud de cada indexador; límites de peticiones.
- Clientes: Transmission, Deluge, rTorrent, aria2, SABnzbd, NZBGet (Download Station ya está en la fase 1).
- Emby como servidor multimedia.

### Fase 5: Subtítulos (M) ← equivalente a Bazarr
- Proveedores de subtítulos (OpenSubtitles y otros), idiomas elegidos, se guardan junto al
  archivo; búsqueda automática tras importar y reintentos.

### Fase 6: Música (L) ← equivalente a Lidarr
- MusicBrainz + Cover Art Archive: artistas, álbumes, pistas.
- Parser de releases de audio (FLAC/MP3, bitrate), reglas de calidad de audio.
- Importación y etiquetado básico; avisar al servidor multimedia.

### Fase 7: Avisos e importación desde otras apps (M)
- Avisos: Discord, correo, Telegram, ntfy, Gotify, Pushover, webhook.
- Importar configuración y biblioteca desde Sonarr/Radarr/Lidarr/Prowlarr/Bazarr/Overseerr,
  qBittorrent y SABnzbd (solo lectura sobre las apps viejas).

### Fase 8: Operación y seguridad (M)
- Secretos cifrados con clave maestra; copia de seguridad y restauración desde la web.
- Registros visibles en la app, limpieza de historial, actualización de esquema segura.
- Revisión de seguridad completa antes de exponerlo fuera de casa (recomendaremos Tailscale o
  proxy HTTPS, nunca abrir el puerto).

### Fase 9: App de escritorio (S-M)
- App Tauri (Windows/Linux) que envuelve la misma web y se conecta a tu servidor; en Linux, incluir GStreamer para el vídeo.
- Versiones con instalador por GitHub Actions, como Chunkscope.

## 3b. Riesgos

- **Alcance**: "todo HolyArrs" son meses de trabajo. Cada fase deja una app usable; si algo
  se alarga, se recorta la fase siguiente, no la calidad de la actual.
- **Parser de releases**: es donde más fallan estas apps. Mantendremos un archivo de pruebas
  con cientos de nombres reales y lo ampliaremos con cada fallo que encuentres.
- **Leptos 0.9**: habrá una migración cuando salga estable.
- **Legal**: no incluimos indexadores ni enlaces a contenido; la app solo habla con los
  servicios que configures. Sin nada de la marca de HolyArrs ni de GTA.

## 4. Pruebas

- Unitarias en `core` (parser y reglas con cientos de nombres reales de release).
- Integración del servidor con servidores falsos (Torznab, qBittorrent, TMDB, Jellyfin)
  levantados en el propio test: nada sale a Internet en los tests.
- Interfaz: Playwright en Chromium contra el servidor real, con capturas para revisar el aspecto.

## 5. Decisiones tomadas

| Tema | Decisión |
|---|---|
| Nombre | **Heistarr** (libre en GitHub, crates.io, Docker Hub y npm) |
| Repositorio | Local por ahora; privado, sin licencia pública |
| Tecnología web | **Leptos** (no egui: hace falta vídeo y comportamiento web normal) |
| Tema por defecto | **Miami Neón**; los otros cinco como alternativas |
| Idioma | Interfaz, código y documentación en **inglés** |
| Orden | **Películas primero** (fase 1), luego series (fase 2) |
| Puerto | **6464** |
| Cliente de descargas inicial | **qBittorrent y Synology Download Station** (fase 1) |
| Web | Leptos en el navegador + API JSON interna (sin renderizado en servidor de momento) |
| Tráileres | Vídeo HTML5 / YouTube incrustado en la ficha (fase 3) |
| Escritorio | Tauri envolviendo la web (fase 9) |

## 6. Estado

- **Fase 0 y fase 1 (v0.1)**: hechas en lo esencial. Servidor, cuentas, ajustes cifrados,
  TMDB, Torznab, reglas con idioma de audio, búsqueda interactiva y automática, mejoras,
  lista de bloqueados, qBittorrent, Synology Download Station, importador con hardlink,
  escaneo de la biblioteca existente, aviso a Jellyfin/Emby/Plex, interfaz con seis temas.
  Pendiente de fase 0/1: pruebas de interfaz con Playwright en CI, comprobar la imagen
  Docker en el NAS, y el archivo de cientos de nombres reales para el parser.
