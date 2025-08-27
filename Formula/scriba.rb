class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.8.1"

  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.8.1/scriba-x86_64-apple-darwin"
    sha256 "9efecab5ea78276c5ece620db555852ced1a5e8e0f2adfd3731bce1269184d4b"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.8.1/scriba-aarch64-apple-darwin"
    sha256 "a95f5d22516869531cab4b071d4aca092b1cfcfb75ce720e80483807ce397ccd"
  end

  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
