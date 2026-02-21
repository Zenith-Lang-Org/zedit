# Manual de Usuario de zedit

**zedit** — Editor de texto de consola moderno, escrito en Rust puro.
Versión `0.1.0` | Licencia: GPL-3.0 | Parte del ecosistema Z (Zenith, Zymbol)

---

## Tabla de Contenidos

1. [Introduccion — que es zedit](#1-introduccion--que-es-zedit)
2. [Instalacion y compilacion](#2-instalacion-y-compilacion)
3. [Inicio rapido](#3-inicio-rapido)
4. [La interfaz](#4-la-interfaz)
5. [Referencia completa de atajos de teclado](#5-referencia-completa-de-atajos-de-teclado)
6. [Busqueda y reemplazo](#6-busqueda-y-reemplazo)
7. [Multi-cursor](#7-multi-cursor)
8. [Gestion de paneles (splits)](#8-gestion-de-paneles-splits)
9. [Arbol de archivos](#9-arbol-de-archivos)
10. [Terminal integrado](#10-terminal-integrado)
11. [Integracion LSP](#11-integracion-lsp)
12. [Gutter de Git](#12-gutter-de-git)
13. [Vista Diff](#13-vista-diff)
14. [Minimapa](#14-minimapa)
15. [Sesion y recuperacion ante fallos](#15-sesion-y-recuperacion-ante-fallos)
16. [Referencia de configuracion](#16-referencia-de-configuracion)
17. [Atajos personalizados](#17-atajos-personalizados)
18. [Resaltado de sintaxis y temas](#18-resaltado-de-sintaxis-y-temas)
19. [Sistema de Plugins](#19-sistema-de-plugins)
20. [Solucion de problemas](#20-solucion-de-problemas)
21. [Licencia](#21-licencia)

---

## 1. Introduccion — que es zedit

zedit es un editor de texto de consola moderno escrito completamente en Rust puro, sin dependencias externas de ningun tipo. Solo utiliza la biblioteca estandar de Rust (`std`) mas llamadas directas a la libc mediante FFI. Es parte del ecosistema Z junto con Zenith y Zymbol, y tiene su origen en el proyecto REPL Minilux.

### Filosofia de diseno

zedit fue concebido para ser:

- **Rapido**: arranque en menos de 10 ms, respuesta al teclado en menos de 5 ms.
- **Liviano**: binario de menos de 1 MB (aproximadamente 500 KB despues de hacer `strip`).
- **Moderno**: atajos de teclado familiares (`Ctrl+C`, `Ctrl+V`, `Ctrl+S`, `Ctrl+Z`) en lugar de modos tipo Nano, Vim o Emacs.
- **Independiente**: sin cargo ni crates externos; todo — parser JSON, motor de expresiones regulares, cliente LSP, emulador VT100, sistema de plugins — esta implementado desde cero.
- **Compatible con estandares**: gramaticas TextMate (`.tmLanguage.json`) y temas compatibles con VS Code.

### Caracteristicas principales

- Cero dependencias externas (solo `std` + libc FFI)
- Atajos de teclado modernos al estilo escritorio
- Manejo nativo de UTF-8
- Resaltado de sintaxis con gramaticas TextMate y temas VS Code
- Renderizado diferencial (solo redibuja celdas modificadas)
- Multi-buffer: abre y alterna entre multiples archivos
- Busqueda y reemplazo incremental con resaltado
- Deshacer/rehacer con agrupamiento inteligente por tiempo
- Soporte de raton: clic, arrastrar para seleccionar, scroll
- Auto-indentacion al pulsar Enter
- Comentarios de linea por lenguaje con `Ctrl+/`
- Deteccion adaptativa de color (TrueColor, 256 colores, 16 colores)
- Multi-cursor con seleccion de ocurrencias
- Paneles divisibles (splits horizontales y verticales)
- Arbol de archivos lateral
- Terminal integrado con PTY persistente
- Cliente LSP: autocompletado, hover, ir a definicion, diagnosticos
- Gutter de Git con indicadores de cambios vs HEAD
- Vista Diff lado a lado vs HEAD de Git
- Minimapa con codificacion braille
- Persistencia de sesion y recuperacion ante fallos (swap)
- Sistema de plugins via IPC JSON

---

## 2. Instalacion y compilacion

### Requisitos

- Rust (edicion 2024 o superior). Se recomienda instalar via [rustup](https://rustup.rs/).
- No se necesitan dependencias adicionales del sistema.

### Compilar desde el codigo fuente

```sh
# Clonar el repositorio
git clone https://github.com/Zenith-Lang-Org/zedit.git
cd zedit

# Compilar en modo release (recomendado)
cargo build --release

# Opcional: reducir el binario a ~500 KB
strip target/release/zedit

# Instalar en el PATH del sistema (ejemplo)
sudo cp target/release/zedit /usr/local/bin/zedit
```

### Compilar en modo debug

```sh
cargo build
# El binario se ubica en target/debug/zedit
```

### Verificar la instalacion

```sh
zedit --version   # imprime: zedit 0.1.0
zedit --help      # muestra la ayuda con atajos basicos
```

### Rendimiento esperado

| Metrica | Objetivo |
|---------|---------|
| Arranque | < 10 ms |
| Tecla a pantalla | < 5 ms |
| Abrir archivo de 1 MB | < 50 ms |
| Tamano del binario | < 1 MB |

---

## 3. Inicio rapido

### Abrir zedit

```sh
zedit                  # buffer vacio (o restaura ultima sesion)
zedit archivo.rs       # abrir un archivo
zedit src/main.rs      # rutas relativas o absolutas
```

### Primeros pasos

1. **Escribir**: empieza a escribir directamente. zedit detecta el lenguaje por la extension del archivo y activa el resaltado de sintaxis de forma automatica.

2. **Guardar**: `Ctrl+S`. Si el buffer no tiene nombre, zedit pedira una ruta. Para guardar con otro nombre usa `Ctrl+Shift+S`.

3. **Abrir otro archivo**: `Ctrl+O`. Se pedira la ruta por teclado.

4. **Navegar entre buffers abiertos**: `Ctrl+PgDn` (siguiente) y `Ctrl+PgUp` (anterior). La barra de pestanas en la parte superior muestra todos los buffers activos.

5. **Buscar texto**: `Ctrl+F`. La busqueda es incremental y resalta las coincidencias en tiempo real.

6. **Deshacer / Rehacer**: `Ctrl+Z` / `Ctrl+Y`. Los caracteres escritos de forma consecutiva se agrupan automaticamente; una pausa de 500 ms inicia un nuevo grupo.

7. **Salir**: `Ctrl+Q`. Si hay cambios sin guardar, zedit lo advertira y pedira confirmacion; pulsa `Ctrl+Q` una segunda vez para salir sin guardar.

8. **Ayuda rapida**: `F1` activa/desactiva el overlay de ayuda con los atajos principales.

---

## 4. La interfaz

La ventana de zedit se divide en varias regiones visibles:

```
┌─────────────────────────────────────────────────────────┐
│ [main.rs] [config.rs*] [Untitled]          ← Barra de pestanas
├────┬────────────────────────────────────────┬────┬──────┤
│    │                                        │    │ Mini │
│ A  │           Area de edicion             │ G  │ mapa │
│ r  │                                        │ u  │      │
│ b  │                                        │ t  │      │
│ o  │                                        │ t  │      │
│ l  │                                        │ e  │      │
│    │                                        │ r  │      │
├────┴────────────────────────────────────────┴────┴──────┤
│ [1] main.rs | Rust | UTF-8 | Ln 42, Col 10 | NORMAL    │
│                                            ← Barra de estado
└─────────────────────────────────────────────────────────┘
```

### Barra de pestanas

Aparece en la parte superior y lista todos los buffers abiertos. El buffer activo se resalta. Un asterisco (`*`) junto al nombre indica que hay cambios sin guardar. Se puede navegar entre pestanas con `Ctrl+PgDn` / `Ctrl+PgUp`. La barra hace scroll automaticamente si hay mas pestanas de las que caben en pantalla.

### Area de edicion (paneles)

Es el espacio principal donde se edita el texto. Puede dividirse en multiples paneles con `Ctrl+\` (horizontal) o `Ctrl+Shift+\` (vertical). Cada panel puede mostrar un buffer diferente o el mismo buffer desde distintas posiciones.

### Gutter izquierdo

La columna izquierda del area de edicion muestra:

- **Numeros de linea** (si `line_numbers: true` en la configuracion).
- **Indicadores de Git**: cambios respecto al HEAD del repositorio:
  - `+` verde — linea añadida
  - `~` amarillo — linea modificada
  - `-` rojo — indica que se elimino una linea en esa posicion

### Barra de estado

La linea inferior muestra informacion contextual:

- Nombre del archivo y lenguaje detectado
- Codificacion (UTF-8)
- Numero de linea y columna del cursor
- Modo actual (edicion normal, busqueda, reemplazo, etc.)
- Mensajes de estado y errores temporales
- Diagnosticos del servidor LSP (errores y advertencias)

### Minimapa

El minimapa aparece en el margen derecho cuando esta activado (`Ctrl+Shift+M`). Muestra una vista comprimida del archivo completo usando caracteres braille Unicode. La region actualmente visible en el editor se resalta con un fondo mas claro. Ocupa 10 columnas de ancho.

### Arbol de archivos

El panel lateral izquierdo (activado con `Ctrl+B`) muestra el arbol de archivos del directorio de trabajo. Permite navegar, expandir/colapsar directorios, crear, renombrar y eliminar archivos, todo desde el teclado.

---

## 5. Referencia completa de atajos de teclado

### 5.1 Archivo

| Atajo | Accion |
|-------|--------|
| `Ctrl+S` | Guardar (pide nombre si el buffer no tiene ruta) |
| `Ctrl+Shift+S` | Guardar Como (siempre pide nueva ruta) |
| `Ctrl+O` | Abrir archivo (pide ruta por teclado) |
| `Ctrl+Q` | Salir (pulsar dos veces si hay cambios sin guardar) |
| `Ctrl+N` | Nuevo buffer vacio |
| `Ctrl+W` | Cerrar buffer actual |
| `Ctrl+PgDn` | Ir al buffer siguiente |
| `Ctrl+PgUp` | Ir al buffer anterior |

### 5.2 Edicion

| Atajo | Accion |
|-------|--------|
| `Ctrl+Z` | Deshacer |
| `Ctrl+Y` | Rehacer |
| `Ctrl+C` | Copiar seleccion (o linea completa si no hay seleccion) |
| `Ctrl+X` | Cortar seleccion (o linea completa si no hay seleccion) |
| `Ctrl+V` | Pegar |
| `Ctrl+Shift+D` | Duplicar linea actual |
| `Ctrl+Shift+K` | Eliminar linea actual |
| `Tab` | Indentar seleccion (o insertar espacios en la posicion del cursor) |
| `Shift+Tab` | Desidentar |
| `Ctrl+/` | Alternar comentario de linea (segun lenguaje) |
| `Enter` | Nueva linea con auto-indentacion |

### 5.3 Navegacion

| Atajo | Accion |
|-------|--------|
| Teclas de flecha | Mover cursor una posicion |
| `Inicio` / `Fin` | Ir al inicio / fin de la linea actual |
| `Ctrl+Inicio` / `Ctrl+Fin` | Ir al inicio / fin del archivo |
| `PgUp` / `PgDn` | Desplazar una pagina arriba / abajo |
| `Ctrl+G` | Ir a numero de linea (muestra prompt) |

### 5.4 Busqueda

| Atajo | Accion |
|-------|--------|
| `Ctrl+F` | Abrir busqueda incremental |
| `Ctrl+H` | Abrir busqueda y reemplazo |
| `F3` | Ir a la siguiente coincidencia |
| `Shift+F3` | Ir a la coincidencia anterior |
| `Ctrl+R` | (Dentro del modo busqueda) Activar/desactivar modo expresiones regulares |

### 5.5 Seleccion

| Atajo | Accion |
|-------|--------|
| `Shift+Flechas` | Extender la seleccion en la direccion indicada |
| `Ctrl+A` | Seleccionar todo el contenido del buffer |
| `Ctrl+L` | Seleccionar la linea completa donde esta el cursor |
| `Ctrl+D` | Seleccionar la siguiente ocurrencia de la seleccion actual (multi-cursor) |
| `Ctrl+Shift+L` | Seleccionar todas las ocurrencias a la vez (multi-cursor) |
| `Alt+Clic` | Agregar un cursor adicional en la posicion clickeada |
| `Escape` | Colapsar todos los cursores a un cursor unico |

### 5.6 Paneles (Splits)

| Atajo | Accion |
|-------|--------|
| `Ctrl+\` | Dividir panel activo horizontalmente (lado a lado) |
| `Ctrl+Shift+\` | Dividir panel activo verticalmente (arriba/abajo) |
| `Ctrl+Shift+W` | Cerrar el panel activo |
| `Alt+Izquierda` | Enfocar el panel a la izquierda |
| `Alt+Derecha` | Enfocar el panel a la derecha |
| `Alt+Arriba` | Enfocar el panel de arriba |
| `Alt+Abajo` | Enfocar el panel de abajo |
| `Alt+Shift+Izquierda` | Reducir el ancho del panel activo |
| `Alt+Shift+Derecha` | Aumentar el ancho del panel activo |
| `Alt+Shift+Arriba` | Reducir la altura del panel activo |
| `Alt+Shift+Abajo` | Aumentar la altura del panel activo |

### 5.7 Vista

| Atajo | Accion |
|-------|--------|
| `F1` | Alternar overlay de ayuda con atajos |
| `Alt+Z` | Alternar ajuste de linea suave (word wrap) |
| `Ctrl+B` | Alternar visibilidad del arbol de archivos lateral |
| `Ctrl+P` | Abrir paleta de comandos (busqueda difusa de todos los comandos) |
| `Ctrl+T` | Alternar panel de terminal integrado |
| `Ctrl+Shift+T` | Abrir nueva pestana de terminal |
| `Ctrl+Shift+M` | Alternar minimapa |

### 5.8 LSP (Language Server Protocol)

| Atajo | Accion |
|-------|--------|
| `Ctrl+Space` | Mostrar menu de autocompletado LSP |
| `Alt+K` | Mostrar popup de documentacion hover |
| `F12` | Ir a la definicion del simbolo bajo el cursor |

> En el menu de autocompletado: `Tab` o `Enter` para insertar la seleccion, `Escape` para cerrar. En el popup de hover: cualquier tecla lo cierra.

### 5.9 Vista Diff (comparar vs HEAD de Git)

| Atajo | Accion |
|-------|--------|
| `F7` | Abrir vista diff del buffer actual comparado con HEAD |
| `F8` | Saltar al siguiente hunk cambiado |
| `Shift+F8` | Saltar al hunk anterior cambiado |
| `Arriba` / `Abajo` | Desplazar la vista diff linea a linea |
| `PgUp` / `PgDn` | Desplazar la vista diff por paginas |
| `Escape` | Cerrar la vista diff y volver al editor |

### 5.10 Raton

| Accion del raton | Efecto |
|------------------|--------|
| Clic simple | Posicionar el cursor en esa ubicacion |
| Clic y arrastrar | Seleccionar texto |
| Doble clic | Seleccionar la palabra bajo el cursor |
| Rueda de desplazamiento | Hacer scroll por el documento |
| `Alt+Clic` | Agregar un cursor adicional (multi-cursor) |

### 5.11 Terminal integrado

| Atajo | Accion |
|-------|--------|
| `Ctrl+T` | Alternar la visibilidad del panel de terminal |
| `Ctrl+Shift+T` | Abrir un nuevo terminal en una pestana separada |
| `Shift+PgUp` / `Shift+PgDn` | Desplazar el historial del terminal |
| `Ctrl+Q` | Salir del foco del terminal y volver al editor |

---

## 6. Busqueda y reemplazo

### Busqueda incremental

Presiona `Ctrl+F` para abrir el modo de busqueda. La busqueda es **incremental**: a medida que escribes, zedit resalta todas las coincidencias en el documento y salta a la primera de ellas.

- Las coincidencias se muestran resaltadas en todo el documento.
- La barra de estado muestra el numero de coincidencias encontradas.
- Presiona `Enter`, `F3` o la flecha abajo para ir a la siguiente coincidencia.
- Presiona `Shift+F3` o la flecha arriba para ir a la coincidencia anterior.
- Presiona `Escape` para cerrar el modo de busqueda y volver al editor.

La busqueda no distingue entre mayusculas y minusculas por defecto.

### Modo expresiones regulares

Dentro del modo de busqueda, presiona `Ctrl+R` para activar el modo de expresiones regulares. El motor de regex de zedit es un subconjunto del estilo Oniguruma, implementado desde cero (NFA/bytecode con backtracking). Admite el 95 %+ de los patrones usados en gramaticas reales de TextMate.

Patrones admitidos habituales:

```
.         Cualquier caracter
\d        Digito (0-9)
\w        Caracter de palabra (letra, digito, _)
\s        Espacio en blanco
[abc]     Clase de caracteres
(foo|bar) Alternacion
foo*      Cero o mas
foo+      Uno o mas
foo?      Cero o uno
^         Inicio de linea
$         Fin de linea
```

### Busqueda y reemplazo

Presiona `Ctrl+H` para abrir el modo de busqueda y reemplazo.

1. Escribe el patron de busqueda en el primer campo.
2. Pulsa `Tab` para pasar al campo de reemplazo.
3. Escribe el texto de reemplazo.
4. Pulsa `Enter` para reemplazar la coincidencia actual, o usa la opcion de reemplazar todas.
5. Pulsa `Escape` para cancelar y cerrar.

---

## 7. Multi-cursor

zedit soporta edicion con multiples cursores simultaneos, util para refactorizar o editar varios lugares a la vez.

### Agregar cursores

| Metodo | Descripcion |
|--------|-------------|
| `Ctrl+D` | Selecciona la siguiente ocurrencia del texto actualmente seleccionado y agrega un cursor ahi. Repite para seguir agregando. |
| `Ctrl+Shift+L` | Selecciona **todas** las ocurrencias del texto seleccionado de una vez. |
| `Alt+Clic` | Coloca un cursor adicional en la posicion del clic. |

### Edicion multi-cursor

Cuando hay multiples cursores activos, cualquier tecla que pulses (texto, borrar, indentar, etc.) se aplica a todos los cursores de forma simultanea.

### Colapsar cursores

Presiona `Escape` para descartar todos los cursores adicionales y volver a un cursor unico.

### Flujo tipico de uso

1. Selecciona una palabra o identificador con doble clic o `Shift+Flechas`.
2. Presiona `Ctrl+D` repetidas veces hasta tener seleccionadas todas las ocurrencias que deseas modificar.
3. Escribe el nuevo texto. Todos los cursores editan a la vez.
4. Presiona `Escape` al terminar.

---

## 8. Gestion de paneles (splits)

zedit permite dividir el area de edicion en multiples paneles independientes. Cada panel puede mostrar un buffer distinto o el mismo buffer desde posiciones diferentes.

### Dividir paneles

| Atajo | Resultado |
|-------|-----------|
| `Ctrl+\` | Divide el panel activo en dos, lado a lado (split horizontal) |
| `Ctrl+Shift+\` | Divide el panel activo en dos, uno sobre otro (split vertical) |

Al dividir, el nuevo panel muestra el mismo buffer que el panel de origen. Puedes abrir un archivo diferente en el nuevo panel con `Ctrl+O`.

### Navegar entre paneles

| Atajo | Accion |
|-------|--------|
| `Alt+Izquierda` | Enfocar el panel a la izquierda del activo |
| `Alt+Derecha` | Enfocar el panel a la derecha del activo |
| `Alt+Arriba` | Enfocar el panel de arriba |
| `Alt+Abajo` | Enfocar el panel de abajo |

El panel activo tiene el cursor visible y recibe todas las pulsaciones de teclado.

### Redimensionar paneles

| Atajo | Accion |
|-------|--------|
| `Alt+Shift+Izquierda` | Mueve el divisor hacia la izquierda (reduce el panel activo) |
| `Alt+Shift+Derecha` | Mueve el divisor hacia la derecha (amplia el panel activo) |
| `Alt+Shift+Arriba` | Mueve el divisor hacia arriba (reduce la altura del panel activo) |
| `Alt+Shift+Abajo` | Mueve el divisor hacia abajo (amplia la altura del panel activo) |

### Cerrar un panel

`Ctrl+Shift+W` cierra el panel activo. Si es el ultimo panel, zedit no lo cerrara (se necesita al menos un panel). Para cerrar el buffer usa `Ctrl+W`.

---

## 9. Arbol de archivos

El arbol de archivos es un panel lateral que muestra la estructura de directorios del proyecto. Se activa y desactiva con `Ctrl+B`.

### Navegacion en el arbol

| Tecla | Accion |
|-------|--------|
| `Arriba` / `Abajo` | Mover el cursor por la lista de nodos |
| `Enter` o `Derecha` | Abrir archivo o expandir directorio |
| `Izquierda` | Colapsar directorio expandido |
| `/` | Activar modo de filtro (busqueda rapida por nombre) |
| `Escape` | Salir del modo filtro o cerrar el arbol |

### Operaciones sobre archivos

| Tecla | Accion |
|-------|--------|
| `n` | Crear nuevo archivo (pide nombre) |
| `d` | Crear nuevo directorio (pide nombre) |
| `r` | Renombrar el archivo/directorio seleccionado |
| `Delete` | Eliminar el archivo/directorio (pide confirmacion) |

### Directorios ignorados por defecto

El arbol omite automaticamente las siguientes entradas comunes:

- `.git`
- `target`
- `node_modules`
- `.DS_Store`
- `__pycache__`

Puedes agregar rutas adicionales a ignorar mediante la opcion `filetree_ignored` en la configuracion.

### Configurar el arbol de archivos

En `~/.config/zedit/config.json`:

```json
{
  "filetree_width": 30,
  "filetree_ignored": ["dist", "build", ".cache"]
}
```

- `filetree_width`: ancho en columnas del panel lateral (rango 15–60, por defecto 30).
- `filetree_ignored`: lista de nombres de archivos o directorios a ocultar.

---

## 10. Terminal integrado

zedit incluye un terminal completamente funcional con PTY (pseudo-terminal) persistente y emulador VT100, accesible sin salir del editor.

### Abrir y usar el terminal

| Atajo | Accion |
|-------|--------|
| `Ctrl+T` | Alternar la visibilidad del panel de terminal |
| `Ctrl+Shift+T` | Abrir una nueva pestana de terminal independiente |

El terminal ejecuta el shell del sistema (`$SHELL`, o el configurado en `terminal_shell`). La sesion PTY es **persistente**: si ocultas el panel con `Ctrl+T` y lo vuelves a abrir, el proceso del shell sigue ejecutandose con su historial intacto.

### Navegar el historial del terminal

| Atajo | Accion |
|-------|--------|
| `Shift+PgUp` | Desplazar el historial del terminal hacia arriba |
| `Shift+PgDn` | Desplazar el historial del terminal hacia abajo |

El numero de lineas de historial es configurable con `terminal_scrollback` (por defecto 1000 lineas, maximo 100 000).

### Volver al editor

Para devolver el foco al editor sin cerrar el terminal, presiona `Ctrl+Q` mientras el terminal esta activo. Esto **no** termina el proceso del shell, solo transfiere el foco de vuelta al area de edicion.

### Configurar el terminal

En `~/.config/zedit/config.json`:

```json
{
  "terminal_shell": "/bin/zsh",
  "terminal_scrollback": 5000
}
```

- `terminal_shell`: ruta absoluta al shell a ejecutar. Si esta vacio, se usa `$SHELL` o `/bin/sh`.
- `terminal_scrollback`: numero maximo de lineas de historial del terminal (100–100 000).

---

## 11. Integracion LSP

zedit tiene soporte nativo para el Language Server Protocol (LSP), lo que permite obtener autocompletado inteligente, documentacion hover, ir a la definicion y diagnosticos de errores para cualquier lenguaje que tenga un servidor LSP disponible.

### Configurar servidores LSP

Los servidores LSP se configuran en `~/.config/zedit/config.json` bajo la clave `lsp`:

```json
{
  "lsp": {
    "rust": {
      "command": "rust-analyzer"
    },
    "python": {
      "command": "pylsp"
    },
    "typescript": {
      "command": "typescript-language-server",
      "args": ["--stdio"]
    },
    "go": {
      "command": "gopls"
    }
  }
}
```

Cada entrada mapea un identificador de lenguaje (debe coincidir con el `name` en la definicion del lenguaje) a la configuracion del servidor:

- `command`: nombre del ejecutable del servidor LSP (debe estar en el `PATH`).
- `args`: lista opcional de argumentos adicionales para el servidor.

El cliente LSP de zedit se lanza de forma diferida (lazy): el servidor solo se inicia la primera vez que abres un archivo del lenguaje correspondiente.

### Autocompletado

Presiona `Ctrl+Space` para solicitar sugerencias de autocompletado al servidor LSP. Aparece un menu desplegable con las opciones disponibles.

- `Tab` o `Enter`: insertar la opcion seleccionada.
- `Arriba` / `Abajo`: navegar por las opciones.
- `Escape`: cerrar el menu sin insertar nada.

### Documentacion hover

Presiona `Alt+K` para mostrar la documentacion del simbolo que esta bajo el cursor. Aparece un popup con la firma del tipo y la documentacion disponible. Cualquier tecla cierra el popup.

### Ir a la definicion

Presiona `F12` para saltar a la definicion del simbolo bajo el cursor. Si la definicion esta en el mismo archivo, el cursor se mueve directamente. Si esta en otro archivo, ese archivo se abre en el panel activo.

### Diagnosticos

El servidor LSP envia diagnosticos (errores y advertencias de compilacion o analisis) de forma asincroona. zedit los muestra en dos lugares:

- **Gutter**: un indicador de color en la columna izquierda de la linea afectada.
- **Barra de estado**: el mensaje del diagnostico mas relevante en la linea donde esta el cursor.

Los diagnosticos se actualizan automaticamente cada vez que guardas el archivo o tras un breve tiempo de inactividad, segun la implementacion del servidor LSP.

### Servidores LSP recomendados por lenguaje

| Lenguaje | Servidor | Instalacion |
|----------|----------|-------------|
| Rust | `rust-analyzer` | `rustup component add rust-analyzer` |
| Python | `pylsp` | `pip install python-lsp-server` |
| TypeScript/JavaScript | `typescript-language-server` | `npm install -g typescript-language-server typescript` |
| Go | `gopls` | `go install golang.org/x/tools/gopls@latest` |
| C/C++ | `clangd` | Incluido con LLVM/Clang |
| Java | `jdtls` | Descargable de eclipse.org |
| PHP | `intelephense` | `npm install -g intelephense` |

---

## 12. Gutter de Git

zedit lee los objetos del repositorio Git directamente desde `.git/` (sin ejecutar el comando `git`) e indica en el gutter izquierdo que lineas han cambiado respecto al commit HEAD actual.

### Indicadores en el gutter

| Simbolo | Color | Significado |
|---------|-------|-------------|
| `+` | Verde | Linea añadida (no existe en HEAD) |
| `~` | Amarillo | Linea modificada respecto a HEAD |
| `-` | Rojo | Indica que se elimino una linea en esa posicion |

Los indicadores se actualizan automaticamente cada vez que editas el buffer o guardas el archivo.

### Requisitos

- El archivo debe estar dentro de un repositorio Git.
- zedit implementa DEFLATE (RFC 1951), lectura de objetos Git (commits, trees, blobs) y el algoritmo de diff Myers completamente en Rust puro, sin dependencias externas.
- Si el repositorio usa objetos empaquetados (packfiles) muy grandes, puede haber limitaciones. Para la mayoria de proyectos funciona sin configuracion adicional.

---

## 13. Vista Diff

La vista Diff permite comparar el contenido actual del buffer con la version almacenada en HEAD de Git, mostrando los cambios lado a lado.

### Abrir la vista diff

Presiona `F7` para abrir la vista diff del buffer activo. Se abre una vista de pantalla completa con:

- **Panel izquierdo**: version del archivo en HEAD (original).
- **Panel derecho**: version actual del buffer (modificada).
- Las lineas añadidas, eliminadas y modificadas se resaltan con colores distintos.

### Navegar por los hunks

Un **hunk** es un bloque contiguo de cambios.

| Atajo | Accion |
|-------|--------|
| `F8` | Saltar al siguiente hunk |
| `Shift+F8` | Saltar al hunk anterior |
| `Arriba` / `Abajo` | Desplazar la vista linea a linea |
| `PgUp` / `PgDn` | Desplazar la vista por paginas |

### Cerrar la vista diff

Presiona `Escape` para cerrar la vista diff y volver al editor normal.

---

## 14. Minimapa

El minimapa es una vista comprimida del archivo completo que aparece en el margen derecho del editor. Usa caracteres **braille Unicode** (rango `U+2800`–`U+28FF`) para representar la densidad del texto, lo que permite visualizar la estructura del codigo de un vistazo.

### Activar y desactivar

`Ctrl+Shift+M` alterna la visibilidad del minimapa.

### Interpretacion

- Cada caracter braille representa un bloque de 2 columnas × 4 filas del codigo fuente.
- Las celdas con codigo (caracteres no blancos) se muestran con un gris claro.
- Las celdas vacias (lineas en blanco) se muestran con un tono muy oscuro.
- La region del archivo actualmente visible en el editor se resalta con un fondo ligeramente mas claro.

El minimapa escala automaticamente para que el archivo completo quepa en la altura disponible del terminal, independientemente del numero de lineas.

---

## 15. Sesion y recuperacion ante fallos

### Persistencia de sesion

zedit guarda automaticamente la sesion al salir. La sesion incluye:

- Lista de archivos abiertos (rutas y buffers sin nombre).
- Posicion del cursor en cada buffer (linea y columna).
- Posicion de scroll (linea superior visible) de cada buffer.
- Buffer activo al momento de salir.

La sesion se almacena en `~/.local/state/zedit/sessions/` (o en `$XDG_STATE_HOME/zedit/sessions/` si la variable esta definida). Cada directorio de trabajo tiene su propio archivo de sesion identificado por un hash del directorio.

**Restauracion automatica**: la proxima vez que ejecutes `zedit` sin argumentos desde el mismo directorio, la sesion anterior se restaura automaticamente. Si abres zedit con un archivo como argumento (`zedit archivo.rs`), la sesion no se restaura.

### Archivos Swap (recuperacion ante fallos)

zedit escribe archivos swap cada **2 segundos** mientras editas, lo que protege tu trabajo en caso de un corte de luz, cierre inesperado del terminal u otro fallo.

**Ubicacion de los archivos swap**:

- Para archivos con nombre: junto al archivo original, con el prefijo `.` y la extension `.swp`. Por ejemplo, `/home/usuario/proyecto/.foo.rs.swp`.
- Para buffers sin nombre: en `~/.local/state/zedit/swap/NewBuffer01.swp`, etc.

**Recuperacion**:

Al abrir un archivo que tiene un swap huerfano (creado por un proceso zedit que ya no existe), zedit detecta la situacion y ofrece la opcion de recuperar el contenido no guardado. Puedes aceptar para restaurar los cambios o rechazar para abrir la version guardada en disco.

Los archivos swap se eliminan automaticamente cuando guardas el archivo o cierras el buffer de forma normal.

**Estados del swap**:

| Estado | Descripcion |
|--------|-------------|
| `None` | No hay swap; el archivo esta limpio |
| `OwnedByUs` | El swap es de esta instancia de zedit; es normal |
| `Orphaned` | El swap lo creo un proceso que ya no existe; posible recuperacion |
| `Corrupt` | El swap esta daanado; se ignora |

---

## 16. Referencia de configuracion

El archivo de configuracion principal es `~/.config/zedit/config.json`. Si no existe, zedit usa todos los valores por defecto. El archivo es JSON estandar; las claves desconocidas se ignoran silenciosamente.

### Ejemplo completo

```json
{
  "tab_size": 4,
  "use_spaces": true,
  "theme": "zedit-dark",
  "line_numbers": true,
  "auto_indent": true,
  "word_wrap": false,
  "filetree_width": 30,
  "filetree_ignored": ["dist", "build", ".cache"],
  "terminal_shell": "",
  "terminal_scrollback": 1000,
  "lsp": {
    "rust": { "command": "rust-analyzer" },
    "python": { "command": "pylsp" },
    "typescript": {
      "command": "typescript-language-server",
      "args": ["--stdio"]
    }
  },
  "languages": [
    {
      "name": "ruby",
      "extensions": ["rb", "rake", "gemspec"],
      "grammar": "ruby.tmLanguage.json",
      "comment": "#"
    }
  ],
  "keybindings": {
    "save": "Ctrl+S",
    "toggle_minimap": "Ctrl+Shift+M"
  }
}
```

### Referencia de opciones

| Clave | Tipo | Por defecto | Descripcion |
|-------|------|-------------|-------------|
| `tab_size` | entero | `4` | Numero de espacios por nivel de indentacion (rango 1–16) |
| `use_spaces` | booleano | `true` | `true` = usar espacios para indentar; `false` = usar caracter de tabulacion |
| `theme` | cadena | `"zedit-dark"` | Nombre del tema de color. Integrados: `zedit-dark`, `zedit-light` |
| `line_numbers` | booleano | `true` | Mostrar el gutter con numeros de linea |
| `auto_indent` | booleano | `true` | Preservar el nivel de indentacion al pulsar Enter |
| `word_wrap` | booleano | `false` | Ajuste suave de lineas largas (no inserta saltos reales) |
| `filetree_width` | entero | `30` | Ancho en columnas del panel del arbol de archivos (rango 15–60) |
| `filetree_ignored` | array de cadenas | `[]` | Nombres de archivos/directorios adicionales a ocultar en el arbol |
| `terminal_shell` | cadena | `""` | Ruta al shell del terminal integrado. Vacio = usa `$SHELL` |
| `terminal_scrollback` | entero | `1000` | Lineas de historial del terminal (rango 100–100 000) |
| `lsp` | objeto | `{}` | Mapa de `language_id` a configuracion de servidor LSP |
| `languages` | array | `[]` | Definiciones de lenguaje de usuario (sobreescribe las integradas por nombre) |
| `keybindings` | objeto | `{}` | Mapa de `nombre_accion` a cadena de atajo personalizado |

### Configurar lenguajes personalizados

La clave `languages` acepta un array de objetos con la siguiente estructura:

```json
{
  "name": "ruby",
  "extensions": ["rb", "rake", "gemspec"],
  "grammar": "ruby.tmLanguage.json",
  "comment": "#"
}
```

| Campo | Requerido | Descripcion |
|-------|-----------|-------------|
| `name` | Si | Identificador unico del lenguaje (en minusculas) |
| `extensions` | Si | Lista de extensiones de archivo (sin el punto inicial) |
| `grammar` | Si | Nombre del archivo de gramatica `.tmLanguage.json` |
| `comment` | No | Prefijo de comentario de linea para `Ctrl+/` |

Si el `name` coincide con uno de los lenguajes integrados, la definicion de usuario lo sobreescribe completamente. Si es un nombre nuevo, se añade a los integrados.

### Variables de entorno

| Variable | Efecto |
|----------|--------|
| `COLORTERM=truecolor` o `COLORTERM=24bit` | Habilitar color verdadero de 24 bits |
| `TERM=xterm-256color` | Habilitar modo de 256 colores |
| `HOME` | Directorio base para localizar la configuracion y los plugins |
| `XDG_STATE_HOME` | Directorio base para sesiones y swap (sustituye a `~/.local/state`) |
| `SHELL` | Shell usado por el terminal integrado si `terminal_shell` esta vacio |

---

## 17. Atajos personalizados

Todos los atajos predeterminados de zedit pueden reasignarse mediante la clave `keybindings` en la configuracion.

### Formato

```json
{
  "keybindings": {
    "nombre_accion": "Modificador+Tecla"
  }
}
```

El formato de la cadena de atajo es:

```
[Ctrl+][Alt+][Shift+]<tecla>
```

Teclas reconocidas: letras y numeros (`A`–`Z`, `0`–`9`), `Enter`, `Tab`, `Backspace`, `Delete`, `Escape`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PgUp`, `PgDn`, `F1`–`F12`, `` ` `` (backtick), `\` (backslash), `/` (slash).

### Ejemplo

```json
{
  "keybindings": {
    "save": "Ctrl+S",
    "diff_open_vs_head": "F7",
    "lsp_complete": "Ctrl+Space",
    "toggle_minimap": "Ctrl+Shift+M",
    "toggle_terminal": "F10"
  }
}
```

> Al redefinir un atajo, el atajo anterior para esa accion queda desactivado automaticamente.

### Lista completa de nombres de acciones

| Nombre de accion | Descripcion |
|------------------|-------------|
| `save` | Guardar archivo |
| `save_as` | Guardar como |
| `open_file` | Abrir archivo |
| `quit` | Salir del editor |
| `new_buffer` | Nuevo buffer vacio |
| `close_buffer` | Cerrar buffer actual |
| `undo` | Deshacer |
| `redo` | Rehacer |
| `duplicate_line` | Duplicar linea |
| `delete_line` | Eliminar linea |
| `toggle_comment` | Alternar comentario de linea |
| `unindent` | Desidentar seleccion |
| `copy` | Copiar |
| `cut` | Cortar |
| `paste` | Pegar |
| `select_all` | Seleccionar todo |
| `select_line` | Seleccionar linea |
| `select_next_occurrence` | Seleccionar siguiente ocurrencia |
| `select_all_occurrences` | Seleccionar todas las ocurrencias |
| `find` | Buscar |
| `replace` | Buscar y reemplazar |
| `find_next` | Siguiente coincidencia |
| `find_prev` | Coincidencia anterior |
| `go_to_line` | Ir a numero de linea |
| `next_buffer` | Buffer siguiente |
| `prev_buffer` | Buffer anterior |
| `split_horizontal` | Dividir horizontalmente |
| `split_vertical` | Dividir verticalmente |
| `close_pane` | Cerrar panel activo |
| `focus_left` | Enfocar panel izquierdo |
| `focus_right` | Enfocar panel derecho |
| `focus_up` | Enfocar panel superior |
| `focus_down` | Enfocar panel inferior |
| `resize_pane_left` | Reducir ancho del panel |
| `resize_pane_right` | Ampliar ancho del panel |
| `resize_pane_up` | Reducir altura del panel |
| `resize_pane_down` | Ampliar altura del panel |
| `toggle_help` | Alternar overlay de ayuda |
| `toggle_wrap` | Alternar ajuste de linea |
| `toggle_file_tree` | Alternar arbol de archivos |
| `command_palette` | Abrir paleta de comandos |
| `toggle_terminal` | Alternar terminal integrado |
| `new_terminal` | Nueva pestana de terminal |
| `lsp_complete` | Menu de autocompletado LSP |
| `lsp_hover` | Documentacion hover LSP |
| `lsp_go_to_def` | Ir a la definicion LSP |
| `diff_open_vs_head` | Abrir vista diff vs HEAD |
| `diff_next_hunk` | Siguiente hunk en la vista diff |
| `diff_prev_hunk` | Hunk anterior en la vista diff |
| `toggle_minimap` | Alternar minimapa |

---

## 18. Resaltado de sintaxis y temas

### Gramaticas TextMate

zedit usa gramaticas TextMate en formato `.tmLanguage.json` para el resaltado de sintaxis. Las gramaticas estan integradas en el binario en tiempo de compilacion mediante `include_str!`, por lo que no necesitas instalar nada adicional.

#### Lenguajes incluidos de forma integrada

| Lenguaje | Extensiones |
|----------|-------------|
| Rust | `.rs` |
| C | `.c`, `.h` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp` |
| Go | `.go` |
| Java | `.java` |
| JavaScript | `.js`, `.mjs` |
| TypeScript | `.ts`, `.tsx` |
| Python | `.py` |
| PHP | `.php` |
| Julia | `.jl` |
| R | `.r`, `.R` |
| JSON | `.json` |
| TOML | `.toml` |
| YAML | `.yaml`, `.yml` |
| Markdown | `.md`, `.markdown` |
| Shell/Bash | `.sh`, `.bash` |
| HTML | `.html`, `.htm` |
| CSS | `.css` |
| XML | `.xml` |
| Zenith | `.zen` |
| Zymbol | `.zym` |
| Minilux | `.mlx` |

#### Gramaticas de usuario

Puedes agregar tus propias gramaticas copiando el archivo `.tmLanguage.json` a:

```
~/.config/zedit/grammars/
```

zedit las descubre automaticamente al arrancar y les da prioridad sobre las integradas si tienen el mismo nombre de archivo.

Tambien puedes registrar gramaticas de usuario a traves de la clave `languages` en la configuracion.

### Temas de color

Los temas siguen el formato de temas de VS Code (JSON con tokenColors). zedit soporta:

- **TrueColor (24 bits)**: cuando `COLORTERM=truecolor` o `COLORTERM=24bit`.
- **256 colores**: cuando `TERM=xterm-256color`. zedit hace degradacion automatica del color mas cercano.
- **16 colores**: modo de compatibilidad maxima para terminales basicos.

#### Temas integrados

| Nombre | Descripcion |
|--------|-------------|
| `zedit-dark` | Tema oscuro por defecto |
| `zedit-light` | Tema claro |

#### Temas de usuario

Copia el archivo de tema JSON compatible con VS Code a:

```
~/.config/zedit/themes/
```

Luego activa el tema en la configuracion:

```json
{
  "theme": "mi-tema"
}
```

El nombre es el nombre del archivo sin la extension `.json`.

### Como funciona el resaltado internamente

1. zedit detecta el lenguaje segun la extension del archivo.
2. Carga la gramatica correspondiente usando el parser JSON propio (~300 lineas, cero dependencias).
3. El tokenizador con estado (`LineState`) procesa las lineas visibles y lleva el estado de construcciones multilinea (strings, comentarios de bloque) entre lineas.
4. Los tokens se mapean a colores del tema mediante jerarquia de scopes.
5. El renderizador diferencial emite solo las secuencias ANSI necesarias para las celdas que han cambiado.
6. Cuando editas, el tokenizador se recalcula desde la linea modificada hacia abajo hasta que el `LineState` converge con el estado en cache.

---

## 19. Sistema de Plugins

zedit soporta plugins externos que se comunican con el editor mediante un protocolo IPC basado en JSON delimitado por saltos de linea sobre stdin/stdout. El runtime soportado actualmente es **Minilux** (`.mlx`).

### Estructura de un plugin

Los plugins se ubican en `~/.config/zedit/plugins/`. Cada plugin es un subdirectorio con al menos dos archivos:

```
~/.config/zedit/plugins/
  miplugin/
    manifest.json
    main.mlx
```

#### manifest.json

```json
{
  "name": "miplugin",
  "version": "1.0.0",
  "description": "Descripcion breve del plugin",
  "main": "main.mlx"
}
```

| Campo | Requerido | Descripcion |
|-------|-----------|-------------|
| `name` | Si | Nombre unico del plugin |
| `version` | No | Version (por defecto `0.1.0`) |
| `description` | No | Descripcion corta (se muestra en la paleta de comandos) |
| `main` | Si | Ruta relativa al script de entrada (relativa al directorio del plugin) |

### Ciclo de vida

1. Al arrancar, zedit escanea `~/.config/zedit/plugins/` y lee los `manifest.json`.
2. Para cada plugin descubierto, lanza `minilux <ruta_al_script>` como proceso hijo.
3. Se comunica con el plugin via stdin/stdout usando JSON (una linea por mensaje).
4. Al salir, zedit envia una señal de cierre a todos los plugins.

> Para que los plugins funcionen, el ejecutable `minilux` debe estar disponible en el `PATH`.

### API de mensajes IPC

Los mensajes del plugin al editor son objetos JSON con los campos `method` (obligatorio) y `params` (obligatorio). Los mensajes con `id` esperan una respuesta.

#### Plugin → Editor

**RegisterCommand** — registra un comando en la paleta:

```json
{
  "id": 1,
  "method": "RegisterCommand",
  "params": {
    "id": "miplugin.formato",
    "label": "Formatear con miplugin",
    "keybinding": "Ctrl+Shift+F"
  }
}
```

**SubscribeEvent** — suscribirse a eventos del editor:

```json
{
  "method": "SubscribeEvent",
  "params": { "event": "buffer_save" }
}
```

Eventos disponibles: `buffer_open`, `buffer_save`, `buffer_close`, `cursor_move`, `text_change`.

**GetBufferText** — solicitar el contenido completo del buffer activo:

```json
{ "id": 2, "method": "GetBufferText", "params": {} }
```

**GetFilePath** — solicitar la ruta del archivo del buffer activo:

```json
{ "id": 3, "method": "GetFilePath", "params": {} }
```

**InsertText** — insertar texto en la posicion del cursor:

```json
{
  "method": "InsertText",
  "params": { "text": "// generado por miplugin\n" }
}
```

**ShowMessage** — mostrar un mensaje en la barra de estado:

```json
{
  "method": "ShowMessage",
  "params": {
    "text": "Formateo completado",
    "kind": "info"
  }
}
```

Valores validos de `kind`: `"info"`, `"warning"`, `"error"`.

#### Editor → Plugin

**Notificacion de evento** (cuando el plugin esta suscrito):

```json
{
  "method": "event",
  "params": {
    "kind": "buffer_save",
    "data": { "path": "/ruta/al/archivo.rs" }
  }
}
```

**Notificacion de comando invocado** (cuando el usuario ejecuta el comando desde la paleta):

```json
{
  "method": "command_invoked",
  "params": { "command_id": "miplugin.formato" }
}
```

**Respuesta a una solicitud** (tras `GetBufferText` o `GetFilePath`):

```json
{
  "id": 2,
  "result": "contenido completo del buffer..."
}
```

### Ejemplo minimo de plugin en Minilux

```
# main.mlx — plugin de ejemplo
# Registra un comando y formatea el buffer al invocarlo

RegisterCommand({ "id": "fmt.run", "label": "Formatear buffer" })
SubscribeEvent({ "event": "buffer_save" })

loop:
  msg = read_message()
  if msg.method == "command_invoked":
    text = GetBufferText()
    # ... procesar el texto ...
    InsertText({ "text": text_formateado })
    ShowMessage({ "text": "Listo", "kind": "info" })
```

---

## 20. Solucion de problemas

### El resaltado de sintaxis no funciona

- Verifica que la extension del archivo este registrada para el lenguaje. Consulta la tabla de lenguajes en la seccion 18.
- Si usas una gramatica de usuario, asegurate de que el archivo `.tmLanguage.json` es un JSON valido y esta en `~/.config/zedit/grammars/`.
- Comprueba que la clave `grammar` en la definicion del lenguaje apunta al nombre de archivo correcto (incluyendo la extension).

### El servidor LSP no responde

- Asegurate de que el ejecutable del servidor LSP esta instalado y accesible en el `PATH`:
  ```sh
  which rust-analyzer
  which pylsp
  ```
- Verifica que el `language_id` en la configuracion de zedit coincide exactamente con el nombre del lenguaje definido en la seccion `languages` (sensible a mayusculas).
- Algunos servidores LSP requieren que el proyecto tenga una estructura especifica (por ejemplo, `Cargo.toml` para Rust, `package.json` para Node). Abre zedit desde el directorio raiz del proyecto.
- Consulta los logs del servidor LSP si tiene opcion de depuracion.

### No se detecta el color verdadero

- Establece la variable de entorno antes de lanzar zedit:
  ```sh
  COLORTERM=truecolor zedit archivo.rs
  ```
  O agrégala permanentemente a tu `~/.bashrc` / `~/.zshrc`.
- Si tu terminal soporta 256 colores pero no TrueColor, usa `TERM=xterm-256color`.

### zedit no restaura la sesion

- La sesion solo se restaura cuando lanzas `zedit` sin argumentos desde el mismo directorio donde terminaste la sesion anterior.
- Si la sesion esta corrupta, elimina el archivo correspondiente en `~/.local/state/zedit/sessions/`.
- zedit solo restaura sesiones con version `1`; sesiones de versiones distintas se descartan.

### Archivo swap huerfano no se detecta

- Los swaps huerfanos para archivos con nombre se detectan cuando abres ese archivo especifico.
- Los swaps huerfanos para buffers sin nombre se detectan al lanzar `zedit` sin argumentos.
- Si el proceso zedit que creo el swap sigue en ejecucion (aunque sea en otro terminal), el swap se considera activo y no se ofrecera recuperacion.

### El arbol de archivos no muestra mis archivos

- Comprueba que el directorio de trabajo de zedit es el correcto. zedit muestra el arbol a partir del directorio desde donde fue lanzado.
- Verifica que el archivo o directorio no esta en la lista `filetree_ignored` de la configuracion o en los ignorados por defecto (`.git`, `target`, `node_modules`, etc.).

### Problemas de rendimiento con archivos muy grandes

- zedit usa un gap buffer optimizado para archivos de hasta ~50 MB.
- El resaltado de sintaxis se calcula solo para las lineas visibles.
- Si notas lentitud, desactiva el minimapa (`Ctrl+Shift+M`) y el arbol de archivos (`Ctrl+B`) para reducir el trabajo de renderizado.

### Caracteres Unicode no se muestran correctamente

- Asegurate de que tu terminal esta configurado para usar UTF-8. La mayoria de terminales modernos lo hacen por defecto.
- zedit maneja texto UTF-8 de forma nativa. Los problemas de visualizacion suelen ser del emulador de terminal, no del editor.

### El terminal integrado no abre

- Verifica que el shell configurado existe y es ejecutable:
  ```sh
  ls -la $(echo $SHELL)
  ```
- Si `terminal_shell` esta vacio en la configuracion, zedit usa `$SHELL`. Si esa variable tampoco esta definida, intenta `/bin/sh`.
- Algunos entornos muy restringidos pueden no soportar la asignacion de PTY. En ese caso, el terminal integrado no estara disponible.

---

## 21. Licencia

zedit esta licenciado bajo la **GNU General Public License version 3.0 (GPL-3.0)**.

Eres libre de:

- **Usar** el programa para cualquier proposito.
- **Estudiar** como funciona y modificarlo para adaptarlo a tus necesidades.
- **Redistribuir** copias.
- **Distribuir** versiones modificadas bajo los mismos terminos de la GPL-3.0.

El texto completo de la licencia se encuentra en el archivo `LICENSE` en la raiz del repositorio, y tambien esta disponible en:

```
https://www.gnu.org/licenses/gpl-3.0.html
```

---

## Apendice: Rutas y archivos de referencia

| Ruta | Descripcion |
|------|-------------|
| `~/.config/zedit/config.json` | Configuracion principal del editor |
| `~/.config/zedit/grammars/` | Gramaticas TextMate del usuario (`.tmLanguage.json`) |
| `~/.config/zedit/themes/` | Temas de color compatibles con VS Code (`.json`) |
| `~/.config/zedit/plugins/` | Directorio de plugins (subdirectorios con `manifest.json`) |
| `~/.local/state/zedit/sessions/` | Archivos de sesion por directorio de proyecto |
| `~/.local/state/zedit/swap/` | Archivos swap de buffers sin nombre |
| `./.filename.ext.swp` | Archivo swap junto al archivo original (para archivos con nombre) |

> Si la variable `XDG_STATE_HOME` esta definida, las rutas bajo `~/.local/state/` usan ese directorio como base en su lugar.

---

*Manual generado para zedit 0.1.0 — Ecosistema Z (Zenith, Zymbol, Minilux)*
