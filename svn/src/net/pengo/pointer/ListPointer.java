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

/**
 * ListPointer.java
 *
 * A pointer to an item in a ListResource
 *
 * @author Peter Halasz
 */

package net.pengo.pointer;

import net.pengo.resource.Resource;
import net.pengo.resource.IntResource;
import net.pengo.resource.ListResource;


public class ListPointer extends Resource
{
	//fixme: should this subtype SmartPointer ?
	
    public final SmartPointer list = new JavaPointer("net.pengo.resource.ListResource");
    public final SmartPointer index = new JavaPointer("net.pengo.resource.IntResource");
	
	public ListPointer(ListResource list, IntResource index) {
        super();
        
        list.addSink(this);
        list.setName("List");
        setList(list);
        
        index.addSink(this);
        index.setName("Index");
        setIndex(index);
	}
	
	public Resource[] getSources()
	{
		return new Resource[] { list, index };
	}
	
	public Resource evaluate() {
		return getList().elementAt(getIndex());
	}
	
	public ListResource getList() {
		return ((ListResource)list.evaluate());
	}
	
	public void setList(ListResource list) {
		this.list.setValue(list);
	}
	
	public IntResource getIndex() {
		return (IntResource)index.evaluate();
	}

	public void setIndex(IntResource index) {
		this.index.setValue(index);
	}
	
    public boolean isReference() {
		//fixme: i guess it's a reference?
        return true;
    }
	
    public boolean isPrimative() {
        return false;
    }
}

