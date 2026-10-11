-- Run as a PostgreSQL administrator after Platform's `01-database-and-roles.sql` and before the first
-- ChatGPT archive start (XR-021 CONTRACTS.md R3-11). Set the password separately with
-- `\password ratatoskr_chatgpt`; no credential belongs in this repository. The service creates and
-- owns the `chatgpt_archive` schema inside the shared `ratatoskr` database and needs nothing else.

do $$
begin
    if not exists (select 1 from pg_roles where rolname = 'ratatoskr_chatgpt') then
        create role ratatoskr_chatgpt login;
    end if;
end
$$;

alter role ratatoskr_chatgpt nosuperuser nocreatedb nocreaterole noreplication nobypassrls;
grant connect, create on database ratatoskr to ratatoskr_chatgpt;

\connect ratatoskr
revoke all on schema public from ratatoskr_chatgpt;
