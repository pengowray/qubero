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
 * Created on Dec 30, 2003
 */
package net.pengo.resource;

import java.awt.event.ActionEvent;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.Collections;
import java.util.List;

import javax.swing.AbstractAction;
import javax.swing.JMenu;

import net.pengo.pointer.QFunction;
import net.pengo.pointer.SmartPointer;

/**
 * @author Peter Halasz
 */
public class TypeRegistry {
    static private TypeRegistry singleton;
    static public TypeRegistry instance(){
	if (singleton == null) {
	    singleton = new TypeRegistry();
	    // singleton.add(new BooleanAddressedResource().type()); // use this syntax when all types are implemented nicely and shit
	    singleton.add(new JavaType(BooleanAddressedResource.class));
	    singleton.add(new JavaType(IntAddressedResource.class));
	    singleton.add(new JavaType(MagicNumberResource.class));
	    singleton.add(new JavaType(StringAddressedResource.class));
	    singleton.add(new JavaType(ListAddressedResource.class));
	    singleton.add(new JavaType(SimpleSelectionResource.class));
	}
	return singleton;
    }
    
    private List registered = new ArrayList();
    
    public void add(TypeResource t) {
	registered.add(t);
    }
    
    private TypeRegistry() {
	super();
    }
    
    public TypeResource[] registeredTypeList() {
	return (TypeResource[])registered.toArray(new TypeResource[0]);
    }
    
    public QFunction[] selectionQFunctionList() {
	//FIXME: dodgy shit
	
	//return new QFunction[] {};
	
	TypeResource[] r = registeredTypeList();
	List q = new ArrayList();
	
	for (int i=0; i<r.length; i++) {
	    QFunction[] cons = r[i].getConstructorsForSelections();
	    if (cons != null && cons.length > 0) {
		q.addAll(Arrays.asList(cons));
	    }
	}
	
	return (QFunction[])q.toArray(new QFunction[0]);
	
	//        return new QFunction[] {
	//                getConstructorsForSelections(BooleanAddressedResource.class)[0],
	//                getConstructorsForSelections(IntAddressedResource.class)[0],
	//                getConstructorsForSelections(MagicNumberResource.class)[0],
	//                getConstructorsForSelections(StringAddressedResource.class)[0],
	//                //getConstructorsForSelections(JavaPointer.class)[0]
	//        };
    }
    
    public void giveConversionActions(JMenu menu, SelectionResource sel) {
	//FIXME: should accept AddressedResources too
	
	QFunction[] qf = selectionQFunctionList();
	//menu.add(new JSeparator());
	//System.out.println("adding..!" + qf.length);
	for (int i = 0; i < qf.length; i++) {
	    final QFunction function = qf[i];
	    final SelectionResource selection = sel;
	    
	    if (function != null) {
		//System.out.println("adding.. " + function.getName());
		AbstractAction aa = new AbstractAction(function.getName()) {

		    public void actionPerformed(ActionEvent e) {
			//System.out.println("function:" + function.getName());
			//System.out.println("function TypeName():" + function.getTypeName());
			//System.out.println("function eval:" + function.evaluate());
			SmartPointer sp = function.invoke(null, new Resource[]{selection});
			Resource qr = sp.evaluate(); //FIXME: shouldn't be needed
			selection.getOpenFile().getDefinitionList().add(qr);
			//FIXME: do or dont do?
			qr.editProperties(); // do, for now
		    }
		};
		menu.add(aa);
	    }
	}
	
    }
}
