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
 * Created on Jan 16, 2004
 */
package net.pengo.resource;

import net.pengo.pointer.QFunction;

/**
 * @author Peter Halasz
 *
 * */
public abstract class TypeResource extends Resource {

    public TypeResource() {
        super();
        // TODO Auto-generated constructor stub
    }

    public Resource[] getSources() {
        // TODO Auto-generated method stub
        //return null;
		return new Resource[0];
    }

    public abstract boolean canConvertTo(TypeResource type, Aspect aspect);
    public abstract boolean canConvertTo(TypeResource x);
    
    // can convert there and back again, without loss in "aspect".
    public abstract boolean canIsomorphicConvert(TypeResource x, Aspect aspect);
    
    public abstract QFunction[] getFunctions();
    
    public abstract QFunction[] getConstructors();
    abstract public QFunction[] getConstructorsForSelections();
    
    //isDynamic() ?
    
    public abstract boolean equals(TypeResource tr);


}
