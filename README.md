# PORT Plugins

Colección de plugins oficiales para [PORT](https://github.com/loonbac/port) (Plugin-Oriented Rust Terminal).

## Plugins disponibles

- **`transparency` (`port-plugin-transparency`)**: Ajusta la opacidad del fondo de la ventana para habilitar transparencias y efectos de desenfoque en compositores Wayland (Niri, Hyprland, etc.).
- **`font` (`port-plugin-font`)**: Configuración declarativa de tipografía (`family`), tamaño base y fuentes de respaldo.
- **`font-zoom` (`port-plugin-font-zoom`)**: Zoom interactivo de fuente con atajos estándar (`Ctrl++`, `Ctrl+-`, `Ctrl+0`) recalculando en vivo la geometría del PTY.
- **`shortcuts` (`port-plugin-shortcuts`)**: Gestor de atajos de teclado personalizados (`bind`, `bind_str`) asociados a acciones y callbacks.

## Desarrollo

```bash
# Iniciar entorno de desarrollo
nix-shell

# Ejecutar pruebas de los plugins
cargo test
```
