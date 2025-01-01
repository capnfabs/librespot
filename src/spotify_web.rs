use std::{env, sync::{Arc, Mutex}};

use chrono::{TimeDelta, Utc};
use librespot_core::session::Session;
use log::{info, error};
use rspotify::{Token as RspotifyToken, AuthCodeSpotify, clients::OAuthClient, model::{AlbumId, parse_uri, PlaylistId, PlayContextId, Type, PlayableId, TrackId}};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

const SCOPE: &str = "user-modify-playback-state,user-read-playback-state";

pub struct WebApi {
    token: Arc<Mutex<Option<RspotifyToken>>>,
    device_name: String,
    session: Session,
}

impl WebApi {
    pub fn new(session: Session, device_name: String) -> WebApi {
        WebApi {
            token: Arc::new(Mutex::new(None)),
            device_name,
            session,
        }
    }

    pub async fn open_uri(&self, uri: &str, shuffle: bool) -> Result<(), Box<dyn std::error::Error>> {
        let device_name = utf8_percent_encode(&self.device_name, NON_ALPHANUMERIC).to_string();
        let token = self.ensure_fresh_token().await.ok_or("No token")?;
        let sp = create_spotify_api(token);
        let devices = sp.device().await;
        let device_id = devices.unwrap_or_else(|_err| vec![])
            .into_iter()
            .find(|d| d.name == device_name)
            .map(|device| device.id).flatten();
        if uri.contains("spotify:track") {
            let playable_track: PlayableId = TrackId::from_uri(uri)?.into();
            // This is just one song so don't mess with shuffle
            sp.start_uris_playback([playable_track], device_id.as_deref(), None, None).await?;
        } else {
            let (id_type, _id) = parse_uri(uri)?;
            let id: PlayContextId = match id_type {
                Type::Album => AlbumId::from_id_or_uri(uri)?.into(),
                Type::Playlist => PlaylistId::from_id_or_uri(uri)?.into(),
                _default => {
                    Err(format!("Couldn't resolve URI {uri}"))?
                }
            };
            // This is a set of songs so mess with shuffle
            info!("Setting shuffle: {shuffle:?}");
            sp.shuffle(shuffle, device_id.as_deref()).await?;
            sp.start_context_playback(id, device_id.as_deref(), None, None).await?;
        };
        Ok(())
    }

    pub async fn ensure_fresh_token(&self) -> Option<RspotifyToken> {
        {
            // This is in a block so that it gets dropped if we don't return
            let token = self.token.lock().unwrap();
            if let Some(t) = token.as_ref() {
                if !t.is_expired() {
                    return Some(t.clone());
                };
            }
        }

        // token needs refresh
        let token = refresh_token(&self.session).await;
        info!("Got new token, I think?");
        let mut token_store = self.token.lock().unwrap();
        *token_store = token.clone();
        info!("Stored token");
        token
    }
}

fn create_spotify_api(token: RspotifyToken) -> AuthCodeSpotify {
    AuthCodeSpotify::from_token(token)
}

async fn refresh_token(sess: &Session) -> Option<RspotifyToken> {
    info!("Requesting new token");
    let client_id = env::var("LIBRESPOT_CLIENT_ID").expect("Please set the LIBRESPOT_CLIENT_ID env variable");
    let token_result = sess.token_provider().get_token_with_client_id(SCOPE, &client_id).await;
    match token_result {
        Ok(token) => {
            let expires_in = token.expires_in;
            let mut rspot_tok = RspotifyToken::default();
            rspot_tok.access_token = token.access_token;
            rspot_tok.expires_at = Some(Utc::now() + expires_in);
            rspot_tok.expires_in = TimeDelta::from_std(expires_in).expect("Token expiry out of bounds");
            info!("Got token");
            Some(rspot_tok)
        }
        Err(e) => {
            error!("Got error trying to load token: {:?}", e);
            None
        }
    }
}
