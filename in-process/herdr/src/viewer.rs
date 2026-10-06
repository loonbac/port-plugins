//! Visor de solo lectura del hilo VIVO de una sesión de subagente.
//!
//! Responsabilidad única: convertir la identidad de una sesión de presencia en
//! el comando que sigue su hilo publicado. Nada más: no conoce la sidebar, ni
//! las pestañas, ni el estado del plugin.
//!
//! # Qué sigue
//!
//! El hilo vivo que gentle-pi ya publica en
//! `<presenceDir>/<sessionHash>.<incarnation>.activity.json` (ver
//! [`crate::presence`]). Ese archivo se reescribe con cada actualización del
//! subagente, así que es la única fuente que existe **mientras** el subagente
//! corre. El visor no necesita el registro de tarea (`tasks/<id>.json`, que solo
//! aparece al terminar) ni el `.jsonl` de sesión.
//!
//! # Por qué es de solo lectura
//!
//! `pi --session <archivo>` escribe al abrir, y el archivo de actividad es del
//! publicador. Aquí solo se lee: `jq` no modifica nada, y todos los datos (ruta,
//! hash, incarnación y etiqueta) llegan como argumentos posicionales del shell
//! (`$1..$4`), nunca interpolados en el texto del programa.
//!
//! # Por qué se relee y no se vigila con `tail`
//!
//! El publicador reescribe el archivo entero de forma atómica; `tail -f` sobre
//! un archivo reemplazado por `rename` deja de ver los cambios. Por eso el
//! script relee el archivo cada 3 s y pinta solo los items cuyo índice supera lo
//! ya impreso.

use std::path::Path;

use port_term_core::pty::PtyConfig;

/// Programa `jq` que traduce el archivo de actividad a líneas legibles.
///
/// Se declara como macro para poder empotrarlo dentro de [`SCRIPT`] en tiempo
/// de compilación sin duplicar el texto. No lleva comillas simples: el script lo
/// encierra entre ellas.
///
/// La primera línea es una línea de control para el shell: `@unavailable` si el
/// contrato marcó el hilo como no publicado, o `@count=<items>;<vivo>` con el
/// total de items y si queda alguna tarea en curso. El resto son filas:
/// `text → <texto>` (también para cualquier tipo textual desconocido), en fg-15;
/// `· piensa <texto>` y `· nota <texto>` en dim fg-7; `▸ <nombre> …<estado>` en
/// bold fg-9 con la palabra de estado coloreada, sus salidas en fg-7 (o fg-7
/// sobre bg-0 si la herramienta falló) y un pie dim con el recuento de líneas.
/// La herramienta `read` es la excepción: su cabecera lleva la ruta
/// (`▸ read <ruta> …<estado>`, recortada a 120) y su salida se colapsa a UNA
/// sola fila (`↳ <primera línea> … +N líneas`, sin sufijo cuando no hay más),
/// porque volcar el archivo entero llenaba la vista de decenas de `↳`.
/// Los recortes son 300 para texto y salida, 200 para razonamiento y nota y 60
/// para el nombre de la herramienta; se corta con `…`.
macro_rules! jq_filter {
    () => {
        r#"def ansi($code): "\u001b[" + $code + "m";
def RESET: ansi("0");
def BOLD_FG9: ansi("1;91");
def FG7: ansi("37");
def FG10: ansi("92");
def FG11: ansi("93");
def FG15: ansi("97");
def BG0: ansi("40");
def DIM_FG7: ansi("2") + ansi("37");
def clip($n): tostring | gsub("\u001b\\[[0-9;]*[A-Za-z]"; "") | gsub("[\\n\\r\\t]+"; " ") | gsub("  +"; " ") | if length > $n then .[0:$n] + "…" else . end;
def out_lines($n): gsub("\u001b\\[[0-9;]*[A-Za-z]"; "") | split("\n") | map(gsub("[\\r\\t]+"; " ") | gsub("  +"; " ")) | map(select(length > 0)) | map(if length > $n then .[0:$n] + "…" else . end);
def row: if .kind == "tool" then
  (.running // false) as $running
  | (.isError // false) as $err
  | ((.name // "") | clip(60)) as $name
  | ((.output // "") | out_lines(300)) as $lines
  | (if $running then FG11 + "…corriendo" + RESET
     elif $err then BOLD_FG9 + "…falló" + RESET
     else FG10 + "…ok" + RESET end) as $state
  | if $name == "read" then
      (((.arguments.path // .arguments.file // .arguments // "") | clip(120)) as $path
       | (BOLD_FG9 + "▸ read " + $path + " " + RESET + $state),
         (if ($lines | length) > 0 then
            (if $err then BG0 else "" end) + FG7 + "↳ " + $lines[0]
              + (if ($lines | length) > 1 then " … +" + ((($lines | length) - 1) | tostring) + " líneas" else "" end)
              + RESET
          else empty end))
    else
      (BOLD_FG9 + "▸ " + $name + " " + RESET + $state),
        ($lines[] | (if $err then BG0 else "" end) + FG7 + "↳ " + . + RESET),
        (if $running then DIM_FG7 + "  …corriendo" + RESET
         elif $err then DIM_FG7 + "  ✗ falló · " + (($lines | length) | tostring) + " líneas" + RESET
         else DIM_FG7 + "  ✓ ok · " + (($lines | length) | tostring) + " líneas" + RESET end)
    end
elif .kind == "thinking" then DIM_FG7 + "· piensa " + ((.text // "") | clip(200)) + RESET
elif .kind == "note" then DIM_FG7 + "· nota " + ((.text // "") | clip(200)) + RESET
else FG15 + "text → " + ((.text // "") | clip(300)) + RESET end;
if ((.schema // 0) != 1) or ((.sessionHash // "") != $hash) or ((.incarnation // "") != $incarnation) or ((.activity.tasks? | type) != "array") then error("identidad o forma invalida")
elif ((.unavailable // null) != null) then "@unavailable"
else [.activity.tasks[]? | (.thread.items? // [])[]? | select(type == "object")] as $items
  | [.activity.tasks[]? | (.summary.status? // "")] as $statuses
  | ($statuses | map(select(. == "running" or . == "queued" or . == "waiting")) | length) as $live
  | "@count=\($items | length);\(if $live > 0 then 1 else 0 end)",
    ($items | to_entries[] | select(.key >= $seen) | .value | row)
end"#
    };
}

/// Programa `jq` que traduce el archivo de actividad a líneas legibles.
pub(crate) const FILTER: &str = jq_filter!();

/// Script del shell que relee la actividad y pinta solo lo nuevo.
///
/// Los datos llegan como argumentos posicionales: `$1` directorio de presencia,
/// `$2` hash de sesión, `$3` incarnación, `$4` etiqueta. El shell los trata como
/// datos, nunca como texto del programa.
pub(crate) const SCRIPT: &str = concat!(
    "FILE=\"$1/$2.$3.activity.json\"\n",
    "NAME=\"$4\"\n",
    "RULE=\"$(printf '─%.0s' {1..48})\"\n",
    "SEEN=0\n",
    "FIRST=1\n",
    "STOP_AT=\"\"\n",
    "while :; do\n",
    "  OUT=$(jq -r --unbuffered --arg hash \"$2\" --arg incarnation \"$3\" --argjson seen \"$SEEN\" '",
    jq_filter!(),
    "' \"$FILE\" 2>/dev/null)\n",
    "  CODE=$?\n",
    "  if [ $CODE -ne 0 ]; then\n",
    "    if [ $FIRST -eq 1 ]; then\n",
    "      printf '\\033[1;91m● %s\\033[0m\\n' \"$NAME\"\n",
    "      printf '\\033[2m\\033[37m%s\\033[0m\\n' \"$RULE\"\n",
    "      printf '\\033[2m\\033[37m  sin actividad disponible\\033[0m\\n'\n",
    "      exit 3\n",
    "    fi\n",
    "    printf '\\033[2m\\033[37m  visor cerrado: la actividad dejo de estar disponible\\033[0m\\n'\n",
    "    exit 0\n",
    "  fi\n",
    "  if [ $FIRST -eq 1 ]; then\n",
    "    printf '\\033[1;91m● %s\\033[0m\\n' \"$NAME\"\n",
    "    printf '\\033[2m\\033[37m%s\\033[0m\\n' \"$RULE\"\n",
    "    FIRST=0\n",
    "  fi\n",
    "  CONTROL=$(printf '%s\\n' \"$OUT\" | head -n 1)\n",
    "  ROWS=$(printf '%s\\n' \"$OUT\" | tail -n +2)\n",
    "  case \"$CONTROL\" in\n",
    "    \"@unavailable\")\n",
    "      printf '\\033[2m\\033[37m  el hilo no esta publicado (activity-too-large)\\033[0m\\n'\n",
    "      exit 0\n",
    "      ;;\n",
    "    \"@count=\"*)\n",
    "      COUNTS=${CONTROL#@count=}\n",
    "      LIVE=${COUNTS#*;}\n",
    "      TOTAL=${COUNTS%;*}\n",
    "      if [ -n \"$ROWS\" ]; then printf '%s\\n' \"$ROWS\"; fi\n",
    "      SEEN=$TOTAL\n",
    "      if [ \"$LIVE\" = \"0\" ]; then\n",
    "        if [ -z \"$STOP_AT\" ]; then STOP_AT=$(date +%s); fi\n",
    "        if [ $(( $(date +%s) - STOP_AT )) -ge 15 ]; then\n",
    "          printf '\\033[2m\\033[37m  el subagente termino: visor cerrado\\033[0m\\n'\n",
    "          exit 0\n",
    "        fi\n",
    "      else\n",
    "        STOP_AT=\"\"\n",
    "      fi\n",
    "      ;;\n",
    "  esac\n",
    "  sleep 3\n",
    "done\n"
);

/// Comando que sigue el hilo vivo de una sesión de subagente.
///
/// Se ejecuta como
/// `bash -c <SCRIPT> herdr-view <presenceDir> <sessionHash> <incarnation> <label>`:
/// `herdr-view` es el `$0` (nombre que el shell usa en sus mensajes de error) y
/// los cuatro datos quedan en `$1..$4`, fuera del texto que el shell interpreta.
/// Esta función solo describe el proceso: no abre ni lee el archivo de actividad.
pub fn watch_command(
    presence_dir: &Path,
    session_hash: &str,
    incarnation: &str,
    agent_label: &str,
) -> PtyConfig {
    // El filtro se empotra entre comillas simples dentro del script: si alguna
    // vez llevara una, el shell lo rompería en silencio. La comprobación vive
    // aquí, en el único punto que construye el comando, para que una edición
    // futura del filtro no pueda romper la petición sin que se note.
    debug_assert!(
        !FILTER.contains('\''),
        "el filtro no puede llevar comillas simples"
    );
    debug_assert!(SCRIPT.contains(FILTER), "el script debe empotrar el filtro");

    PtyConfig {
        command: "bash".to_string(),
        args: vec![
            "-c".to_string(),
            SCRIPT.to_string(),
            "herdr-view".to_string(),
            presence_dir.display().to_string(),
            session_hash.to_string(),
            incarnation.to_string(),
            agent_label.to_string(),
        ],
        cwd: None,
    }
}
