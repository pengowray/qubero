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
 * AddressListPage.java
 *
 * a list of existing resources to select from for a resource
 *
 * this is crap. doesn't work anywasy. forget it.
 */

package net.pengo.propertyEditor;
import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.ResourceRegistry;

//fixme: shithouse for non-modal dialogs

public class ResourceListPage extends MethodSelectionPage
{
    private JavaPointer jp;
    
    private static Object[] dodgyNullPointerFix; //
    
    public ResourceListPage(PropertiesForm form, JavaPointer jp) {
        super(form, null, selectionStatic(jp), jp+"");
        this.jp = jp;
    }
    
    static private PropertyPage[] staticPages() {
        //todo
        return null;
    }
    
    static private Object[] selectionStatic(JavaPointer jp) {
        //fixme:
        //System.out.println("jp: " + jp);
        //new Error("Debug").printStackTrace();
        //System.out.println("---");
        if (jp==null)
            return dodgyNullPointerFix;
        
        //Object[] o = ResourceRegistry.instance().getAllOfType(jp.getType()).toArray();
        Object[] o = ResourceRegistry.instance().getAll().toArray();
        dodgyNullPointerFix = o;
        
        return o;
    }
    
    private Object[] selection() {
        return selectionStatic(jp);
    }
    
    public PropertyPage getSelected() {
        
        return new TextOnlyPage("Selection", selection()[selected]+"");
    }
}

