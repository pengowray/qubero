/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

package net.pengo.data;


import java.io.*;

/** 
 * rudimentary file editing. entire file is kept in an array.
 */
public class SmallFileData extends ArrayData {
    protected File file;
    protected String name;

    public SmallFileData(File file) throws IOException {
        super();
	this.file = file;
	name = file.getName();
        readFileToMemory(new FileInputStream(file));
    }	

    public SmallFileData(String filename) throws IOException {
        this(new File(filename));
    }
    
    public SmallFileData(InputStream is, String name) {
        super();
        this.file = null;
        this.name = name;
        readFileToMemory(is);
    }

    /**
    * reads the entire (hex) file to memory for quick access
    */
    protected void readFileToMemory(InputStream fis) {
        //FIXME: very inefficent! fix it sometime. 
        //FIXME: oops. i didn't know about file.length()
        //FIXME: oops! i didn't know about ByteArrayOutputStream
        
        try {
            //File in = file;
            //FileInputStream fis = new FileInputStream(in);

            // read in bytes to data
        	byte[] byteArray = getByteArray();
        	
            int ch; // current byte
            int chc = 0; // char count
            //if (byteArray == null) // byte array now starts life as new byte[0]
            setByteArray(new byte[1024]);
            while ((ch=fis.read()) != -1) {
                if (byteArray.length <= chc) {
                    // double byteArray's size
                    byte[] byteArrayTemp = new byte[byteArray.length*2] ;
                    System.arraycopy(byteArray,0,byteArrayTemp,0,byteArray.length);
                    setByteArray(byteArrayTemp);
                }
                byteArray[chc] = (byte)ch;
                chc++;
            }
            
            // truncate byteArray to correct size
            if (byteArray.length != chc) {
                byte[] byteArrayTemp = new byte[chc] ;
                System.arraycopy(byteArray,0,byteArrayTemp,0,chc);
                setByteArray(byteArrayTemp);
            }
            
        } catch (IOException e) {
            //FIXME: !!!
            setByteArray( new String("error reading file: " + e).getBytes() );
        }
        
    }

    public String toString() {
        return name;
    }

    public String getType() {
        return "small file";
    }
    
    
}
