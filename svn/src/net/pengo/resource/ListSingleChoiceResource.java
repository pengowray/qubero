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
 * ListSingleChoiceResource.java
 *
 * A single choice of an item in a list.
 *
 * Perhaps in future a new, dynamic enum type should be created. but this will do for now.
 *
 * fixme: This should also work as a wrapper for the (IntResource) choice , that this can be used as an IntResource
 *
 * fixme: either that, or have an auto-conversion
 *
 * @author  Peter Halasz
 */

package net.pengo.resource;

import java.util.List;
import net.pengo.pointer.SmartPointer;
import net.pengo.pointer.JavaPointer;
import net.pengo.propertyEditor.PropertyPage;
import net.pengo.propertyEditor.ListSingleChoicePage;

public class ListSingleChoiceResource extends Resource
{
    public final SmartPointer choiceP = new JavaPointer(net.pengo.resource.IntResource.class);
    public final SmartPointer listP = new JavaPointer(net.pengo.resource.ListResource.class);

    public ListSingleChoiceResource(IntResource choice, ListResource list) {
        choiceP.addSink(this);
        choiceP.setName("Choice");
        setChoice(choice);

        listP.addSink(this);
        listP.setName("List");
        setList(list);
	}
	
	public Resource[] getSources()
	{
		return new Resource[] {choiceP, listP};
	}
	
	public IntResource getChoice() {
		return (IntResource)choiceP.evaluate();
	}

	public void setChoice(IntResource choice) {
		choiceP.setValue(choice);
	}
	
	public ListResource getList() {
		return (ListResource)listP.evaluate();
	}

	public void setList(ListResource list) {
		listP.setValue(list);
	}
	
	public PropertyPage getValuePage() {
		return new ListSingleChoicePage(this);
	}
	
    public List getPrimaryPages() {
        List pp = super.getPrimaryPages();
	pp.add(new ListSingleChoicePage(this));
        return pp;
    }	
    
    public String valueDesc() {
	//fixme: maybe ok
        return getList().elementAt(getChoice()).valueDesc();
    }
	
}

