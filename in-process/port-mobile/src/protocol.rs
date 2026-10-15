//! Protocolo entre PORT y el cliente remoto.
//!
//! Única responsabilidad: definir qué viaja por el cable y cómo se calcula lo
//! que cambió. Es puro y no conoce sockets, ni HTTP, ni el núcleo de PORT: los
//! tipos de la rejilla que usa son los del cable, no los de la terminal, así que
//! el servidor es quien traduce y este módulo se prueba solo.
//!
//! El principio que lo gobierna: **sólo viaja lo que cambió**. El ancho de banda
//! depende de lo que se movió en pantalla y no del tamaño de la rejilla, que es
//! lo que hace que un cliente remoto se sienta tan fluido como la terminal
//! local.

use serde::{Deserialize, Serialize};

/// Mensaje del servidor al cliente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Estado completo, al conectar y cuando cambia el modelo de espacios.
    Layout {
        /// Espacios, pestañas y estado de agente de cada sesión.
        layout: Layout,
    },
    /// Lo que cambió de una sesión desde el cuadro anterior.
    Frame {
        /// Sesión a la que pertenece el cuadro.
        session: usize,
        /// Alto total de la rejilla, para que el cliente ajuste el suyo.
        rows: usize,
        /// Ancho total, en columnas.
        columns: usize,
        /// Dónde está el cursor, si está visible.
        ///
        /// Va en su propio campo y no dentro de las filas porque el cursor se
        /// mueve sin que ninguna fila cambie: si viajara con ellas, escribir una
        /// letra obligaría a mandar la fila entera.
        #[serde(default)]
        cursor: Option<CursorView>,
        /// Sólo las filas que cambiaron, con su índice.
        changed: Vec<RowUpdate>,
    },
    /// Una sesión dejó de existir: su pestaña se cerró.
    Closed {
        /// Sesión que ya no está.
        session: usize,
    },
}

/// Mensaje del cliente al servidor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Escribe en una sesión.
    ///
    /// El texto va tal cual y el servidor lo convierte en bytes: una tecla de
    /// control viaja como su secuencia de escape, que es texto válido.
    Input {
        /// Sesión que recibe la entrada.
        session: usize,
        /// Texto a escribir.
        data: String,
    },
    /// Redimensiona la rejilla de una sesión.
    Resize {
        /// Sesión a redimensionar.
        session: usize,
        /// Columnas pedidas.
        columns: usize,
        /// Filas pedidas.
        rows: usize,
    },
    /// Este cliente pasa a mirar otra sesión.
    ///
    /// El foco es del cliente, no de la ventana: mirar otra pestaña desde el
    /// teléfono no cambia la que está en el escritorio.
    Focus {
        /// Sesión que este cliente quiere mirar.
        session: usize,
    },
}

/// Dónde está el cursor dentro de la rejilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorView {
    /// Fila del cursor.
    pub row: usize,
    /// Columna del cursor.
    pub column: usize,
    /// Si está visible; un programa puede esconderlo.
    pub visible: bool,
}

/// Una fila de la rejilla, tal como viaja por el cable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowUpdate {
    /// Índice de la fila dentro de la rejilla.
    pub index: usize,
    /// Tramos de la fila, en orden.
    pub runs: Vec<WireRun>,
}

/// Un tramo de texto con su color.
///
/// Se agrupa el texto por color, como en la terminal: un tramo cuenta como uno
/// solo en el cable por más caracteres que tenga.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireRun {
    /// Texto del tramo.
    pub text: String,
    /// Color de frente, en `#rrggbb`.
    pub fg: String,
    /// Color de fondo, en `#rrggbb`.
    pub bg: String,
}

/// Estado de agente que el cliente dibuja.
///
/// Es el mismo modelo que usa la barra del escritorio, para que el teléfono
/// muestre exactamente los mismos marcadores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentView {
    /// Sin dato: no se dibuja marcador.
    Unknown,
    /// Trabajando.
    Working,
    /// Esperando una respuesta del usuario.
    Blocked,
    /// Terminó sin que el usuario lo viera.
    Idle,
}

/// El modelo de espacios y pestañas, tal como lo ve el cliente.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// Espacios abiertos, en su orden.
    pub spaces: Vec<SpaceView>,
    /// Índice del espacio activo en el escritorio.
    pub active_space: usize,
}

/// Un espacio con sus pestañas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceView {
    /// Nombre visible del espacio.
    pub name: String,
    /// Rama de git, si se conoce.
    pub branch: String,
    /// Color propio del espacio, en `#rrggbb`, si tiene.
    pub color: Option<String>,
    /// Índice de su pestaña activa.
    pub active_tab: usize,
    /// Estado de agente agregado del espacio.
    pub agent: AgentView,
    /// Sus pestañas.
    pub tabs: Vec<TabView>,
}

/// Una pestaña, que es una sesión de terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabView {
    /// Título visible.
    pub title: String,
    /// Sesión que corre en esta pestaña.
    pub session: usize,
    /// Estado de agente de su sesión.
    pub agent: AgentView,
    /// Si el usuario ya vio el estado: lo que ya se vio no reclama atención.
    pub seen: bool,
}

/// Índices de las filas que cambiaron entre dos cuadros.
///
/// Una rejilla de terminal tiene el alto de la pantalla y una fila conserva su
/// índice cuando el contenido se desplaza, porque el desplazamiento ya ocurrió
/// dentro del terminal. Por eso se compara posición a posición y no hace falta
/// buscar corrimientos.
///
/// Un alto distinto marca además todas las filas que aparecen de más: el cliente
/// necesita saber que su rejilla cambió de tamaño, no sólo su contenido.
pub fn changed_rows(previous: &[RowUpdate], current: &[RowUpdate]) -> Vec<usize> {
    let shared = previous.len().min(current.len());
    let mut changed: Vec<usize> = (0..shared)
        .filter(|&index| previous[index] != current[index])
        .collect();
    changed.extend(shared..current.len());
    changed
}

/// Filas que cambiaron, ya recortadas a esos índices.
///
/// Es lo que se manda: la lista de índices sola no sirve al cliente, que
/// necesita el contenido nuevo de cada una.
pub fn changed_slice(previous: &[RowUpdate], current: &[RowUpdate]) -> Vec<RowUpdate> {
    changed_rows(previous, current)
        .into_iter()
        .filter_map(|index| current.get(index).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fila de una sola pieza, para comparar cuadros en las pruebas.
    fn fila(index: usize, texto: &str) -> RowUpdate {
        RowUpdate {
            index,
            runs: vec![WireRun {
                text: texto.to_string(),
                fg: "#c9d1d9".to_string(),
                bg: "#000000".to_string(),
            }],
        }
    }

    /// Un cuadro de tres filas con la del medio cambiada.
    fn cuadro_base() -> Vec<RowUpdate> {
        vec![fila(0, "uno"), fila(1, "dos"), fila(2, "tres")]
    }

    /// Sin cambios no se manda nada.
    #[test]
    fn sin_cambios_no_viaja_nada() {
        let cuadro = cuadro_base();
        assert!(changed_rows(&cuadro, &cuadro).is_empty());
        assert!(changed_slice(&cuadro, &cuadro).is_empty());
    }

    /// Sólo se marca la fila que cambió, no todo el cuadro.
    #[test]
    fn solo_viaja_la_fila_que_cambio() {
        let anterior = cuadro_base();
        let actual = vec![fila(0, "uno"), fila(1, "DOS"), fila(2, "tres")];

        assert_eq!(changed_rows(&anterior, &actual), vec![1]);
        let enviado = changed_slice(&anterior, &actual);
        assert_eq!(enviado.len(), 1);
        assert_eq!(enviado[0].index, 1);
        assert_eq!(enviado[0].runs[0].text, "DOS");
    }

    /// Un cuadro más alto marca las filas que aparecieron.
    #[test]
    fn un_cuadro_mas_alto_marca_lo_nuevo() {
        let anterior = cuadro_base();
        let actual = vec![
            fila(0, "uno"),
            fila(1, "dos"),
            fila(2, "tres"),
            fila(3, "cuatro"),
        ];

        assert_eq!(changed_rows(&anterior, &actual), vec![3]);
    }

    /// Un cuadro más bajo marca las que se fueron, que el cliente debe borrar.
    #[test]
    fn un_cuadro_mas_bajo_marca_lo_que_se_fue() {
        let anterior = cuadro_base();
        let actual = vec![fila(0, "uno"), fila(1, "dos")];

        // Las que sobreviven se comparan; la que desapareció no está en `actual`,
        // así que el cliente se entera del alto nuevo por el mensaje.
        assert!(changed_rows(&anterior, &actual).is_empty());
        assert!(changed_slice(&anterior, &actual).is_empty());
    }

    /// El color cuenta como contenido: repintar una fila la manda de nuevo.
    #[test]
    fn un_cambio_de_color_manda_la_fila() {
        let anterior = cuadro_base();
        let mut actual = cuadro_base();
        actual[2].runs[0].fg = "#ff0000".to_string();

        assert_eq!(changed_rows(&anterior, &actual), vec![2]);
        assert_eq!(changed_slice(&anterior, &actual)[0].runs[0].fg, "#ff0000");
    }

    /// Los mensajes van y vuelven con la forma que espera el cliente.
    #[test]
    fn los_mensajes_respetan_su_forma() {
        let frame = ServerMessage::Frame {
            session: 7,
            rows: 24,
            columns: 80,
            cursor: Some(CursorView {
                row: 3,
                column: 9,
                visible: true,
            }),
            changed: vec![fila(1, "hola")],
        };
        let json = serde_json::to_string(&frame).expect("serializar");
        assert!(json.contains("\"type\":\"frame\""), "{json}");
        assert!(
            json.contains("\"column\":9"),
            "el cursor debe viajar: {json}"
        );
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&json).expect("deserializar"),
            frame
        );

        let input = ClientMessage::Input {
            session: 7,
            data: "\u{1b}[A".to_string(),
        };
        let json = serde_json::to_string(&input).expect("serializar");
        assert!(json.contains("\"type\":\"input\""), "{json}");
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&json).expect("deserializar"),
            input
        );

        let layout = ServerMessage::Layout {
            layout: Layout {
                spaces: vec![SpaceView {
                    name: "port-plugins".to_string(),
                    branch: "master".to_string(),
                    color: None,
                    active_tab: 0,
                    agent: AgentView::Blocked,
                    tabs: vec![TabView {
                        title: "terminal".to_string(),
                        session: 7,
                        agent: AgentView::Blocked,
                        seen: false,
                    }],
                }],
                active_space: 0,
            },
        };
        let json = serde_json::to_string(&layout).expect("serializar");
        assert!(json.contains("\"agent\":\"blocked\""), "{json}");
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&json).expect("deserializar"),
            layout
        );
    }
}
