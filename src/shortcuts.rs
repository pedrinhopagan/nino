use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::export::futures_core::Stream;
use zbus::message::Type;
use zbus::{Connection, DynamicType, MatchRule, MessageStream, OwnedObjectPath};

use crate::Msg;

const COMPONENT: &str = "nino";
const COMPONENT_NAME: &str = "Nino";
const META: i32 = 0x1000_0000;
const SHIFT: i32 = 0x0200_0000;
const KEY_N: i32 = 0x4E;
const KEY_R: i32 = 0x52;
const KEY_S: i32 = 0x53;
const KEY_ESC: i32 = 0x0100_0000;
const FINISH: [&str; 4] = [COMPONENT, "finish", COMPONENT_NAME, "Concluir a gravação"];
const SET_PRESENT_NO_AUTOLOADING: u32 = 2 | 4;
const TAKEOVER_FROM: &str = "org.kde.spectacle.desktop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Screenshot,
    Record,
    Note,
    Finish,
}

const ACTIONS: [(Action, &str, &str, i32); 3] = [
    (
        Action::Screenshot,
        "screenshot",
        "Capturar região",
        META | SHIFT | KEY_S,
    ),
    (Action::Record, "record", "Gravar região", META | KEY_R),
    (Action::Note, "note", "Anotar na gravação", META | KEY_N),
];

pub fn spawn(tx: Sender<Msg>) {
    std::thread::spawn(move || {
        if let Err(e) = zbus::block_on(run(&tx)) {
            eprintln!("nino: shortcuts: {e}");
            let _ = tx.send(Msg::Fatal(format!("shortcuts: {e}")));
        }
    });
}

async fn run(tx: &Sender<Msg>) -> zbus::Result<()> {
    let conn = Connection::session().await?;

    for (_, name, friendly, key) in ACTIONS {
        register(&conn, name, friendly, key).await?;
    }
    call(&conn, "doRegister", &(&FINISH[..],)).await?;
    set_finish(&conn, false).await?;

    let path: OwnedObjectPath = call(&conn, "getComponent", &(COMPONENT,)).await?.body().deserialize()?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.kde.kglobalaccel.Component")
        .member("globalShortcutPressed")
        .path(path)
        .build()?;

    let mut stream = MessageStream::for_match_rule(rule, &conn, None).await?;
    while let Some(msg) = std::future::poll_fn(|cx| std::pin::Pin::new(&mut stream).poll_next(cx)).await {
        let msg = msg?;
        let Ok((component, name, _ts)) = msg.body().deserialize::<(String, String, i64)>() else {
            continue;
        };
        if component != COMPONENT {
            continue;
        }
        let action = match name.as_str() {
            "finish" => Some(Action::Finish),
            name => ACTIONS.iter().find(|(_, n, ..)| *n == name).map(|(a, ..)| *a),
        };
        if let Some(action) = action
            && tx.send(Msg::Shortcut(action)).is_err()
        {
            break;
        }
    }
    Ok(())
}

async fn register(conn: &Connection, name: &str, friendly: &str, key: i32) -> zbus::Result<()> {
    let id = vec![COMPONENT, name, COMPONENT_NAME, friendly];
    call(conn, "doRegister", &(&id,)).await?;

    let owner: Vec<String> = call(conn, "action", &(key,)).await?.body().deserialize()?;
    match owner.first().map(String::as_str) {
        None | Some(COMPONENT) => {}
        Some(TAKEOVER_FROM) => {
            let keys: Vec<i32> = call(conn, "shortcut", &(&owner,)).await?.body().deserialize()?;
            let keep: Vec<i32> = keys.into_iter().filter(|k| *k != key).collect();
            eprintln!("nino: taking {friendly} shortcut from {}", owner[..2].join("/"));
            call(conn, "setForeignShortcut", &(&owner, keep)).await?;
        }
        Some(_) => eprintln!("nino: {friendly} shortcut is also bound to {}", owner[..2].join("/")),
    }

    if !set(conn, &id, key).await? {
        eprintln!("nino: kglobalaccel refused shortcut for {name}");
    }
    Ok(())
}

async fn set(conn: &Connection, id: &[&str], key: i32) -> zbus::Result<bool> {
    let keys: Vec<i32> = call(conn, "setShortcut", &(id, vec![key], SET_PRESENT_NO_AUTOLOADING))
        .await?
        .body()
        .deserialize()?;
    Ok(keys == [key])
}

pub async fn set_finish(conn: &Connection, on: bool) -> zbus::Result<()> {
    let keys: Vec<(Vec<i32>,)> = if on {
        vec![(vec![KEY_ESC, KEY_ESC, 0, 0],)]
    } else {
        Vec::new()
    };
    call(
        conn,
        "setShortcutKeys",
        &(&FINISH[..], keys, SET_PRESENT_NO_AUTOLOADING),
    )
    .await?;
    Ok(())
}

async fn call<B>(conn: &Connection, method: &str, body: &B) -> zbus::Result<zbus::Message>
where
    B: zbus::export::serde::Serialize + DynamicType,
{
    conn.call_method(
        Some("org.kde.kglobalaccel"),
        "/kglobalaccel",
        Some("org.kde.KGlobalAccel"),
        method,
        body,
    )
    .await
}
