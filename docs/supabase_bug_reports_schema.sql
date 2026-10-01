-- 1. Create bug_reports table
CREATE TABLE IF NOT EXISTS public.bug_reports (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at TIMESTAMPTZ DEFAULT now() NOT NULL,
    bug_type TEXT NOT NULL DEFAULT 'hata',
    incident_start TEXT,
    incident_end TEXT,
    description TEXT NOT NULL,
    device_info JSONB NOT NULL DEFAULT '{}'::jsonb,
    os_version TEXT,
    cpu TEXT,
    gpu TEXT,
    ram TEXT,
    blackbox_log TEXT,
    core_log TEXT,
    shell_log TEXT,
    tiling_log TEXT,
    system_log TEXT
);

-- Existing Gemini installations have only blackbox_log + system_log.
ALTER TABLE public.bug_reports ADD COLUMN IF NOT EXISTS core_log TEXT;
ALTER TABLE public.bug_reports ADD COLUMN IF NOT EXISTS shell_log TEXT;
ALTER TABLE public.bug_reports ADD COLUMN IF NOT EXISTS tiling_log TEXT;

-- 2. Enable Row Level Security (RLS)
ALTER TABLE public.bug_reports ENABLE ROW LEVEL SECURITY;

-- The desktop app carries a publishable key. It needs INSERT only.
REVOKE ALL ON public.bug_reports FROM anon, authenticated;
GRANT INSERT ON public.bug_reports TO anon, authenticated;

-- 3. Policy: Allow anyone (anon / public) to submit bug reports
DROP POLICY IF EXISTS "Allow public insert to bug_reports" ON public.bug_reports;
CREATE POLICY "Allow public insert to bug_reports"
ON public.bug_reports
FOR INSERT
TO anon, authenticated
WITH CHECK (true);

-- 4. Policy: Disallow public reads / updates / deletes (Admin only via dashboard)
-- Anon users cannot read reports sent by others, ensuring privacy and security.
DROP POLICY IF EXISTS "Deny public select" ON public.bug_reports;
CREATE POLICY "Deny public select"
ON public.bug_reports
FOR SELECT
TO anon
USING (false);
