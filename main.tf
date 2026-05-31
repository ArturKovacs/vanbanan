provider "aws" {
  region = "eu-central-1"
}

# This is needed to allow us to ssh into the instance.
# Generate a keypair first.
resource "aws_key_pair" "key" {
  key_name   = "nginx-key"
  public_key = file("~/.ssh/id_ed25519.pub")
}

resource "aws_security_group" "sg" {
  name = "nginx-sg"

  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  ingress {
    from_port   = 80
    to_port     = 80
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

data "aws_ami" "al2023" {
  most_recent = true

  owners = ["amazon"]

  filter {
    name   = "name"
    values = ["al2023-ami-*-x86_64"]
  }
}

resource "aws_instance" "nginx" {
  ami                    = data.aws_ami.al2023.id
  instance_type          = "t3.nano"
  key_name               = aws_key_pair.key.key_name
  vpc_security_group_ids = [aws_security_group.sg.id]

  user_data = file("user-data.sh")

  provisioner "remote-exec" {
    inline = ["mkdir -p /home/ec2-user/LIBA"]

    connection {
      type        = "ssh"
      user        = "ec2-user"
      private_key = file("~/.ssh/id_ed25519")
      host        = self.public_ip
    }
  }

  provisioner "file" {
    source      = "dist/"
    destination = "/home/ec2-user/LIBA"

    connection {
      type        = "ssh"
      user        = "ec2-user"
      private_key = file("~/.ssh/id_ed25519")
      host        = self.public_ip
    }
  }

  tags = {
    Name = "single-nginx"
  }

  depends_on = [aws_security_group.sg]
}

output "ip" {
  value = aws_instance.nginx.public_ip
}
