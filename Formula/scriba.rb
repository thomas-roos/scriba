class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.9.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.9.0/scriba-x86_64-apple-darwin"
    sha256 "0dd059964e50ab9992c69e6af806ae1aca454264f04dfc2abba10fe1128c7424"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.9.0/scriba-aarch64-apple-darwin"
    sha256 "0500e0d1887a664dced23b0063a9f5e10495b9677901ded4e630d6d374ad69a6"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
