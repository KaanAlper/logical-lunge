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
    system_log TEXT
);

-- 2. Enable Row Level Security (RLS)
ALTER TABLE public.bug_reports ENABLE ROW LEVEL SECURITY;

-- 3. Policy: Allow anyone (anon / public) to submit bug reports
CREATE POLICY "Allow public insert to bug_reports"
ON public.bug_reports
FOR INSERT
TO anon, authenticated
WITH CHECK (true);

-- 4. Policy: Disallow public reads / updates / deletes (Admin only via dashboard)
-- Anon users cannot read reports sent by others, ensuring privacy and security.
CREATE POLICY "Deny public select"
ON public.bug_reports
FOR SELECT
TO anon
USING (false);
