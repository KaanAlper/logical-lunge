// Paged Everything search shared by the overview's scroll and keyboard paths.
(function (scope) {
  scope.LLFileIcon = hit => {
    if (hit.isDir) return 'folder';
    const ext = hit.name.includes('.') ? hit.name.split('.').at(-1).toLowerCase() : '';
    const groups = [
      ['picture_as_pdf', ['pdf']], ['image', ['png','jpg','jpeg','gif','webp','svg','bmp','ico']],
      ['audio_file', ['mp3','wav','flac','ogg','m4a']], ['movie', ['mp4','mkv','avi','mov','webm']],
      ['folder_zip', ['zip','rar','7z','tar','gz']],
      ['code', ['rs','js','ts','tsx','jsx','py','cs','cpp','h','html','css','json','yaml','yml','ps1']],
      ['apps', ['exe','msi']], ['description', ['doc','docx','txt','md','rtf','odt']],
      ['table_chart', ['xls','xlsx','csv','ods']], ['slideshow', ['ppt','pptx','odp']],
    ];
    return groups.find(([, extensions]) => extensions.includes(ext))?.[0] || 'draft';
  };
  class FileSearchPager {
    constructor(invoke, publish) { this.invoke = invoke; this.publish = publish; this.generation = 0; }
    reset(query, term) {
      this.generation++;
      this.term = term;
      this.state = { query, hits: [], error: '', loading: !!term, next: 0, done: !term };
      this.pending = false;
      this.publish(this.state);
    }
    cancel() { this.generation++; }
    async load() {
      if (!this.state || this.pending || this.state.done) return;
      const generation = this.generation, offset = this.state.next;
      this.pending = true;
      this.state = { ...this.state, loading: true };
      this.publish(this.state);
      try {
        const page = await this.invoke('everything_search_page', { query: this.term, maxResults: 10, offset });
        if (generation !== this.generation) return;
        if (page.offset !== offset) throw new Error('Everything sayfa sırası değişti');
        const seen = new Set(this.state.hits.map(hit => hit.fullPath));
        const hits = [...this.state.hits, ...page.hits.filter(hit => !seen.has(hit.fullPath) && seen.add(hit.fullPath))];
        const next = offset + page.hits.length;
        this.state = { ...this.state, hits, next, loading: false, done: !page.hits.length || next >= page.total };
      } catch (error) {
        if (generation !== this.generation) return;
        this.state = { ...this.state, loading: false, done: true, error: String(error) };
      }
      this.pending = false;
      this.publish(this.state);
    }
  }
  scope.LLFileSearchPager = FileSearchPager;
})(globalThis);
