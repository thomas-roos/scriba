class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.6.2"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.6.2/scriba-x86_64-apple-darwin"
    sha256 "ec6a119941fb49c105c90690663530b95e5adb30513211b58fbd3b9d257c9384"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.6.2/scriba-aarch64-apple-darwin"
    sha256 "378baaaa0b6c077811dc4c88a451ba7b7b62ea8e8c1309bb59b0ca9c7df42719"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
