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
 * ListEndianResource.java
 *
 * Copy of ListNegativeFormatsResource
 *
 * Created on 3 October 2004, 09:52
 */

package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;

/**
 *
 * @author  Peter Halasz
 */
public class ListEndianResource extends ListSingleChoiceResource {
    
    /** Creates a new instance of ListEndianResource */
    public ListEndianResource(IntResource r) {
	super(r, getNegList());
    }    
    
    //fixme: make these SmartPointers too ?
    static public SmartPointer negListP = new JavaPointer(ListPrimativeResource.class);
    static private TypeResource stringType = new JavaType(StringResource.class);

    static {
	ListPrimativeResource negList = new ListPrimativeResource(stringType);
	String[] negStrings = new String[] {"Network byte-order (big endian)","Little endian (Intel)"};
	negList.setCount(new IntPrimativeResource(negStrings.length));
	for (int c=0; c<negStrings.length; c++) {
	    negList.setElement(new IntPrimativeResource(c), new StringPrimativeResource(negStrings[c]));
	}
	
	negListP.setValue(negList);
    }
    
    static private ListResource getNegList() {
	return (ListPrimativeResource)negListP.evaluate();
    }
    
}
