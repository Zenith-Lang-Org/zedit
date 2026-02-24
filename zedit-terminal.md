# Zedit — Terminal Interna: Resumen de Mejoras

Este documento describe todos los bugs corregidos y funcionalidades implementadas
en la terminal interna de zedit durante la sesión de desarrollo.

---

## 1. Decodificación UTF-8 correcta

**Problema:** Los caracteres no-ASCII (acentos, símbolos, emojis) aparecían como
diamantes `??` al ejecutar código desde Zenith con `!` u otras herramientas.

**Causa:** `VTerm::process_normal()` reemplazaba todo byte `>= 0x80` con el
carácter de reemplazo Unicode (U+FFFD) en lugar de ensamblar secuencias
multi-byte.

**Solución:** Se implementó una máquina de estados UTF-8 en `VTerm` con campos
`utf8_buf: Vec<u8>` y `utf8_remaining: u8`. Los bytes de inicio (`0xC0–0xFF`)
inician la acumulación; los bytes de continuación (`0x80–0xBF`) completan el
carácter y lo colocan en la celda correcta.

**Archivo:** `src/vterm.rs`

---

## 2. Tracking de mouse para arrastre (drag)

**Problema:** El mouse sólo detectaba clics pero no arrastre, por lo que no era
posible seleccionar texto con el mouse.

**Causa:** El modo `?1000h` (X10) sólo reporta clics. Para detectar el botón
mantenido durante movimiento se necesita `?1002h` (button-event tracking).

**Solución:** Se añadió `?1002h` en `enable_mouse()` y su contraparte `?1002l`
en `disable_mouse()`.

**Archivo:** `src/terminal.rs`

---

## 3. Selección de texto con mouse (drag)

**Problema:** No existía ningún mecanismo de selección de texto en la terminal.

**Solución:** Se añadieron campos de selección a `VTerm`:

```
sel_anchor: Option<(u16, u16)>   // punto de inicio del drag
sel_active: Option<(u16, u16)>   // punto final (extremo móvil)
```

Métodos nuevos: `set_sel_anchor`, `set_sel_active`, `clear_selection`,
`has_selection`, `sel_range`, `is_cell_selected`, `selection_text`.

- **Click izquierdo:** planta el ancla y limpia la selección anterior.
- **Drag (botón + movimiento):** actualiza el extremo activo.
- **Render:** las celdas dentro del rango se muestran con colores invertidos.
- **Cualquier tecla (no Shift+Arrow):** limpia la selección automáticamente.

**Archivos:** `src/vterm.rs`, `src/editor/view.rs`, `src/editor/mod.rs`

---

## 4. Selección de texto con teclado (Shift+Arrow)

**Problema:** `Shift+Derecha` escribía la letra `C` en la línea de comandos;
`Shift+Izquierda` escribía `D`. Ocurría porque las secuencias `\x1b[1;2C` y
`\x1b[1;2D` se reenviaban al PTY y bash las descartaba dejando el carácter final.

**Solución:** `Shift+Arrow` ahora se intercepta **antes** de llegar al PTY:

- Primera pulsación: el ancla se planta en la posición actual del cursor del
  terminal (teniendo en cuenta el offset de scrollback).
- Cada pulsación siguiente mueve el extremo activo una celda en esa dirección.
- La selección resultante usa el mismo sistema visual que el drag con mouse.

**Archivo:** `src/editor/mod.rs` — método `extend_terminal_selection`

---

## 5. Ctrl+C / Ctrl+V / Ctrl+X inteligentes

**Problema:** Estas teclas ejecutaban las acciones del editor de texto (Copy,
Paste, Cut sobre el buffer) en lugar de operar sobre la terminal.

**Solución:**

| Tecla   | Con selección activa          | Sin selección                     |
|---------|-------------------------------|-----------------------------------|
| Ctrl+C  | Copia al portapapeles + OSC52 | Envía `\x03` (SIGINT) al proceso  |
| Ctrl+V  | —                             | Pega portapapeles al PTY          |
| Ctrl+X  | Copia al portapapeles + OSC52 | Envía `\x03` (o Ctrl+X) al PTY   |

El texto copiado va al portapapeles interno de zedit **y** al portapapeles del
sistema vía secuencia OSC 52.

**Archivo:** `src/editor/mod.rs` — `handle_terminal_meta_key`, `terminal_copy`

---

## 6. Reset de scroll al recibir nueva salida

**Problema:** Al hacer scroll hacia arriba para ver historial, el prompt nuevo
aparecía fuera de la vista; había que hacer scroll manualmente hacia abajo para
ver la respuesta de cada comando.

**Solución:** Al final de `VTerm::scroll_up()` (que se llama cuando llega
contenido nuevo), se fuerza `scroll_offset = 0` para que la vista salte
automáticamente al fondo y muestre el nuevo prompt.

**Archivo:** `src/vterm.rs`

---

## 7. Sincronización de tamaño del PTY al redimensionar

**Problema:** Al ampliar o reducir el panel de la terminal con `Alt+Shift+Arrow`,
el área útil se quedaba fija en el tamaño original (generalmente 17 filas) sin
importar cuánto espacio se asignara.

**Causa:** `resize_active_pane()` llamaba a `resolve_layout()` pero no a
`sync_pty_sizes()`, por lo que el PTY y el `VTerm` nunca recibían el nuevo
tamaño.

**Solución:** Se añadió `self.sync_pty_sizes()` inmediatamente después de
`self.resolve_layout()` en `resize_active_pane()`.

**Archivo:** `src/editor/mod.rs`

---

## 8. Resize preserva el contenido reciente

**Problema:** Al reducir el panel de la terminal, se eliminaban las últimas
líneas de salida (las más recientes, donde está el cursor/prompt) en lugar de
desplazar el contenido.

**Causa:** `VTerm::resize()` copiaba las primeras `new_rows` filas del buffer
antiguo, descartando las del fondo (donde se encuentra el cursor).

**Solución:** Se reimplementó `resize()` con comportamiento estándar de terminal:

- **Shrink (reducir):** las filas del tope (`0..delta`) se empujan al scrollback.
  Las filas del fondo (`delta..old_rows`) se convierten en el nuevo buffer.
  `cursor_row -= delta`.

- **Grow (ampliar):** se jalan líneas del final del scrollback hacia el tope del
  nuevo buffer. El contenido existente baja `pull` filas. `cursor_row += pull`.

El contenido nunca se pierde al redimensionar.

**Archivo:** `src/vterm.rs`

---

## 9. Cursor del prompt sigue al scroll

**Problema:** Al hacer scroll hacia arriba con la rueda del mouse o
`Shift+PageUp`, el cursor hardware (bloque parpadeante del prompt) se quedaba en
su posición original en lugar de desplazarse junto con el contenido.

**Causa:** El cálculo de posición del cursor ignoraba cuántas filas de scrollback
se estaban mostrando encima del buffer live:

```rust
// Antes (incorrecto):
screen_row = pane_y + vt_row
```

**Solución:**

```rust
// Después (correcto):
screen_row = pane_y + scrollback_lines + vt_row
```

Si el cursor queda por debajo del borde inferior del panel (el usuario está
viendo historial antiguo), el cursor hardware simplemente no se mueve — correcto
porque el usuario está leyendo, no escribiendo.

**Archivo:** `src/editor/view.rs`

---

## Resumen de archivos modificados

| Archivo                   | Cambios principales                                      |
|---------------------------|----------------------------------------------------------|
| `src/vterm.rs`            | UTF-8, selección, resize inteligente, reset scroll       |
| `src/terminal.rs`         | `?1002h` para drag del mouse                             |
| `src/editor/mod.rs`       | Ctrl+C/V/X, selección teclado, sync PTY, extend_sel      |
| `src/editor/view.rs`      | Render selección, cursor con offset scrollback            |
