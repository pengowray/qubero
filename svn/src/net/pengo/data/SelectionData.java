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

/*
 * SelectionData.java
 *
 * Created on 18 November 2002, 10:42
 *
 * Creates a Data object by combining a selection and data.
 *
 * clones neither data nor selection.
 *
 * FIXME: ignores holes in the data, squishing all data together. requires new
 * superclass: MaskedData
 *
 */
package net.pengo.data;

import net.pengo.selection.*;


import java.io.*;

/**
 *
 * @author  Peter Halasz
 */
public class SelectionData extends Data {
    //should these be cloned?.. probably, but leave up to client
    LongListSelectionModel lsm;
    Data data;
    
    /** Creates a new instance of SelectionData */
    public SelectionData(LongListSelectionModel lsm, Data data) {
        this.lsm = lsm;
        this.data = data;
    }
    
    public InputStream dataStream() {
        return new SelectionStream();
    }
    
    public long getLength() {
        return lsm.getSelectionIndexes().length;
    }
    
    public long getStart() {
        //FIXME: may return -1.. is that wrong?
        return lsm.getMinSelectionIndex();
    }
    
    class SelectionStream extends InputStream {
	long[] sel = lsm.getSelectionIndexes();
        int pos = 0;
	
        public int read() throws IOException {
	    if (sel.length == 0 || pos >= sel.length) {
		return -1;
	    }
	    long sp = sel[pos];
	    int r = (int)(data.readByteArray(sp,1)[0]) & 0xFF; //FIXME: use a stream from data too.
	    //System.out.println("reading byte "+ sp +" of " + sel.length + " from a selection stream: " + r );
	    pos++;
	    return r;
        }
        
    }
}
