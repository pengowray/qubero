/*
 * Created on Dec 30, 2003
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.resource;

import java.awt.event.ActionEvent;
import java.lang.reflect.Constructor;
import java.util.ArrayList;
import java.util.List;

import javax.swing.AbstractAction;
import javax.swing.JMenu;

import net.pengo.pointer.ConstructorQFunction;
import net.pengo.pointer.QFunction;
import net.pengo.pointer.SmartPointer;

/**
 * @author Smiley
 *
 * To change the template for this generated type comment go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
public class TypeRegistry {
    static private TypeRegistry singleton;
    static public TypeRegistry instance(){
        if (singleton == null) {
            singleton = new TypeRegistry();
        }
        return singleton;
    }
    
    
    private List registered;
    
    private TypeRegistry() {
        super();
    }
    
    
    
    public Class[] registeredTypeList() {
        //FIXME: NYI
        return null;
    }
        
    public QFunction[] selectionQFunctionList() {
        //FIXME: dodgy shit
        return new QFunction[] {
                getConstructorsForSelections(BooleanAddressedResource.class)[0],
                getConstructorsForSelections(IntAddressedResource.class)[0],
                getConstructorsForSelections(MagicNumberResource.class)[0]
        };
    }
    
    public static QFunction[] getQConstructors(Class theClass) {
        //FIXME: this should be part of a QType class
        
        //used to be: may be overriden.. for example to use a static constructor, a factory or a singleton method
        
        //FIXME: should use lazy init
        Constructor[] constructor = theClass.getConstructors();
        ConstructorQFunction[] cfp = new ConstructorQFunction[constructor.length];
        for (int i = 0; i < constructor.length; i++) {
            //FIXME: check that constructor is public
            Constructor con = constructor[i];
            cfp[i]=new ConstructorQFunction();
            //cfp[i].addSink(this);
            cfp[i].setValue(con);
            cfp[i].setName("create " + theClass.getName()); //?
        }
        
        return cfp;
    }     
    
    
    /**
     * 
     * @return an array of constructors that can be used with (just) a selection as an argument. 
     */
    public static QFunction[] getConstructorsForSelections(Class theClass) {
        //FIXME: should be generalised. e.g. name the arguements. make it not just for selections
        
        ArrayList returnList = new ArrayList();
        
        QFunction[] cons = getQConstructors(theClass);
        for (int i = 0; i < cons.length; i++) {
            QFunction pointer = cons[i];
            Class[] callSig = pointer.callSignature();
            if (callSig.length == 1 && callSig[0] == SelectionResource.class) {
                //FIXME: possible compatibility of other types (e.g. subtypes) not checked! might miss some.
                //FIXME: still relies on java's Class class
                returnList.add(pointer);
            }
        }
        return (QFunction[])returnList.toArray(new QFunction[0]);
    }    
    
    public void giveConversionActions(JMenu menu, SelectionResource sel) {
        //FIXME: should accept AddressedResources too
        
        QFunction[] qf = selectionQFunctionList();
        //menu.add(new JSeparator());
        for (int i = 0; i < qf.length; i++) {
            final QFunction function = qf[i];
            final SelectionResource selection = sel;
            AbstractAction aa = new AbstractAction(function.getName()) {
                public void actionPerformed(ActionEvent e) {
                    SmartPointer sp = function.invoke(null, new QNodeResource[]{selection});
                    QNodeResource qr = sp.evalute(); //FIXME: shouldn't be needed
                    selection.getOpenFile().getDefinitionList().add(qr);
                }
            };
            menu.add(aa);
            
        }
            
    }
}
