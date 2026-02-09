class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.15.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.1/scriba-x86_64-apple-darwin"
    sha256 "95e66e7fa30d53b4c1584bd2ae93e81fa8066236ddc11c96ab846dfc73642e7a"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.1/scriba-aarch64-apple-darwin"
    sha256 "98261a3f4122bb0f6d80bedc2679b42e08053f48b4bcf2e8ae89c48824583dda"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
