use sqlx::SqlitePool;

pub struct LibraryStatsRepository;

impl LibraryStatsRepository {
    pub async fn get_main_counters(
        pool: &SqlitePool,
        library_id: i64,
    ) -> Result<(i64, f64, i64, f64, i64), sqlx::Error> {
        sqlx::query_as::<_, (i64, f64, i64, f64, i64)>(
            r#"SELECT
                COUNT(*) AS total_tracks,
                COALESCE(SUM(COALESCE(lt.duration, 0)), 0) AS total_duration,
                COALESCE(SUM(lf.size), 0) AS total_size,
                COALESCE(AVG(NULLIF(lt.bitrate, 0)), 0) AS avg_bitrate,
                COALESCE(SUM(lt.play_count), 0) AS total_play_count
            FROM library_tracks lt
            INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
            WHERE lt.library_id = ?"#,
        )
        .bind(library_id)
        .fetch_one(pool)
        .await
    }

    pub async fn count_albums(pool: &SqlitePool, library_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM library_albums la
             WHERE la.library_id = ?
               AND (NOT EXISTS (SELECT 1 FROM library_tracks lt0 WHERE lt0.library_album_id = la.id)
                    OR EXISTS (SELECT 1 FROM library_tracks lt JOIN library_files lf ON lf.id = lt.file_id WHERE lt.library_album_id = la.id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)))",
        )
        .bind(library_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    }

    /// Les artistes qui ont réellement quelque chose ici. `library_artists`
    /// seule en garde qui n'ont plus ni piste ni album.
    pub async fn count_artists(pool: &SqlitePool, library_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            // Comptage ensembliste, voir `LibraryRepository`.
            r#"SELECT COUNT(*) FROM (
                   SELECT la.artist_id FROM library_artists la
                   WHERE la.library_id = ?1
                   INTERSECT
                   SELECT artist_id FROM (
                       SELECT lt.artist_id FROM library_tracks lt
                       INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
                       WHERE lt.library_id = ?1 AND lt.artist_id IS NOT NULL
                       UNION
                       SELECT lta.artist_id FROM library_track_artists lta
                       INNER JOIN library_tracks lt ON lt.id = lta.library_track_id
                       INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
                       WHERE lta.library_id = ?1
                       UNION
                       SELECT lb.artist_id FROM library_albums lb
                       WHERE lb.library_id = ?1 AND lb.artist_id IS NOT NULL
                         AND (NOT EXISTS (SELECT 1 FROM library_tracks lt0 WHERE lt0.library_album_id = lb.id)
                              OR EXISTS (SELECT 1 FROM library_tracks lt JOIN library_files lf ON lf.id = lt.file_id WHERE lt.library_album_id = lb.id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)))
                   )
               )"#,
        )
        .bind(library_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    }

    pub async fn count_genres(pool: &SqlitePool, library_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(DISTINCT lc.genre) FROM library_cache lc
               INNER JOIN library_tracks lt ON lt.cache_id = lc.id
               INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
               WHERE lt.library_id = ? AND lc.genre IS NOT NULL AND lc.genre != ''"#,
        )
        .bind(library_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    }

    pub async fn get_format_stats(
        pool: &SqlitePool,
        library_id: i64,
    ) -> Vec<(String, i64)> {
        sqlx::query_as::<_, (String, i64)>(
            r#"SELECT UPPER(COALESCE(lc.audio_format, lf.extension)) AS fmt, COUNT(*) AS cnt
               FROM library_tracks lt
               INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
               LEFT JOIN library_cache lc ON lc.id = lt.cache_id
               WHERE lt.library_id = ?
               GROUP BY fmt ORDER BY cnt DESC"#,
        )
        .bind(library_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    }

    pub async fn count_quality_hires(pool: &SqlitePool, library_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM library_tracks lt
               INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
               LEFT JOIN library_cache lc ON lc.id = lt.cache_id
               WHERE lt.library_id = ? AND lc.bits_per_sample >= 24"#,
        )
        .bind(library_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    }

    pub async fn count_quality_lossless(pool: &SqlitePool, library_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM library_tracks lt
               INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
               LEFT JOIN library_cache lc ON lc.id = lt.cache_id
               WHERE lt.library_id = ? AND lc.bits_per_sample = 16
               AND LOWER(lc.audio_format) IN ('flac', 'wav', 'alac', 'aiff')"#,
        )
        .bind(library_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    }

    pub async fn get_top_genres(pool: &SqlitePool, library_id: i64) -> Vec<(String, i64)> {
        sqlx::query_as::<_, (String, i64)>(
            r#"SELECT lc.genre AS name, COUNT(*) AS cnt
               FROM library_cache lc
               INNER JOIN library_tracks lt ON lt.cache_id = lc.id
               INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
               WHERE lt.library_id = ? AND lc.genre IS NOT NULL AND lc.genre != ''
               GROUP BY lc.genre ORDER BY cnt DESC LIMIT 10"#,
        )
        .bind(library_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    }

    pub async fn get_top_artists(pool: &SqlitePool, library_id: i64) -> Vec<(String, i64)> {
        sqlx::query_as::<_, (String, i64)>(
            // Les deux sources : l'artiste porté par la piste, et ceux de la
            // table de liaison, souvent vide.
            r#"SELECT a.name, COUNT(*) AS cnt
               FROM (
                   SELECT lt.artist_id AS artist_id, lt.id AS track_id
                   FROM library_tracks lt
                   INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
                   WHERE lt.library_id = ?1 AND lt.artist_id IS NOT NULL

                   UNION

                   SELECT lta.artist_id, lt.id
                   FROM library_track_artists lta
                   INNER JOIN library_tracks lt ON lt.id = lta.library_track_id
                   INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
                   WHERE lta.library_id = ?1
               ) paires
               INNER JOIN artists a ON a.id = paires.artist_id
               GROUP BY a.id ORDER BY cnt DESC LIMIT 10"#,
        )
        .bind(library_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    }

    pub async fn get_top_played(
        pool: &SqlitePool,
        library_id: i64,
    ) -> Vec<(String, String, i64, Option<String>)> {
        match sqlx::query_as::<_, (String, String, i64, Option<String>)>(
            r#"SELECT
                COALESCE(lt.title, '') AS title,
                COALESCE(NULLIF(a.name, ''), NULLIF(lc.artist, ''), '') AS artist,
                lt.play_count,
                lc.thumbnail_path
               FROM library_tracks lt
               INNER JOIN library_files lf ON lf.id = lt.file_id AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
               LEFT JOIN library_cache lc ON lc.id = lt.cache_id
               LEFT JOIN artists a ON a.id = lt.artist_id
               WHERE lt.library_id = ? AND lt.play_count > 0
               ORDER BY lt.play_count DESC LIMIT 10"#,
        )
        .bind(library_id)
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                log::error!("get_top_played SQL error: {e}");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_top_played_succeeds_against_real_schema() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./SQL/").run(&pool).await.unwrap();
        sqlx::query("INSERT OR IGNORE INTO profil (id, name) VALUES (1, 'Socle')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR IGNORE INTO library (id, profil_id, name) VALUES (1, 1, 'Lib')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO artists (id, name, name_normalized, sort_name) VALUES ('a1', 'Daft Punk', 'daft punk', 'Daft Punk')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO library_dirs (id, library_id, path, name) VALUES ('d1', 1, '/m', 'm')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO library_files (id, library_id, library_dir_id, path, filename, extension, size, status) VALUES ('f1', 1, 'd1', '/m/a.flac', 'a.flac', 'flac', 100, 'indexed')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO library_tracks (id, library_id, file_id, artist_id, title, title_normalized, play_count) VALUES ('t1', 1, 'f1', 'a1', 'One More Time', 'one more time', 7)")
            .execute(&pool)
            .await
            .unwrap();

        let top = LibraryStatsRepository::get_top_played(&pool, 1).await;
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0, "One More Time");
        assert_eq!(top[0].1, "Daft Punk");
        assert_eq!(top[0].2, 7);
    }
}
