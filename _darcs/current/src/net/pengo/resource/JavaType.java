/*
 * Created on Jan 20, 2004
 */
package net.pengo.resource;

import java.lang.reflect.Constructor;
import java.util.ArrayList;

import net.pengo.pointer.ConstructorQFunction;
import net.pengo.pointer.QFunction;

public class JavaType extends TypeResource {
    
    private Class theClass;

    /**
     *
     */
    public JavaType(Class theClass) {
        super();
        this.theClass = theClass;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#canConvertTo(net.pengo.resource.TypeResource, net.pengo.resource.Aspect)
     */
    public boolean canConvertTo(TypeResource type, Aspect aspect) {
        // TODO Auto-generated method stub
        return false;
    }

    public boolean canConvertTo(TypeResource x) {
        // TODO Auto-generated method stub
        return false;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#canIsomorphicConvert(net.pengo.resource.TypeResource, net.pengo.resource.Aspect)
     */
    public boolean canIsomorphicConvert(TypeResource x, Aspect aspect) {
        // TODO Auto-generated method stub
        return false;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.TypeResource#getFunctions()
     */
    public QFunction[] getFunctions() {
        // TODO Auto-generated method stub
        return null;
    }

    
    public QFunction[] getConstructors() {
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
            cfp[i].setName("create " + Resource.shortTypeName(theClass)); //?
        }
        
        return cfp;
    }

    
    /**
     *
     * @return an array of constructors that can be used with (just) a selection as an argument.
     */
    public QFunction[] getConstructorsForSelections() {
        //FIXME: should be generalised. e.g. name the arguements. make it not just for selections
        
        ArrayList returnList = new ArrayList();
        
        QFunction[] cons = getConstructors();
        for (int i = 0; i < cons.length; i++) {
            QFunction pointer = cons[i];
            SignatureTypeResource callSig = pointer.callSignature();
            TypeResource[] param = callSig.getParam();
            //FIXME: ignores compatibility of other types (e.g. subtypes) not checked! might miss some.
            if (param.length == 1 && param[0].equals(new JavaType(SelectionResource.class))) {
                returnList.add(pointer);
            }
        }
        return (QFunction[])returnList.toArray(new QFunction[0]);
    }

    public boolean equals(TypeResource jt) {
    	if (jt instanceof JavaType)
    		return equals((JavaType)jt);
    	
    	return false;
    }
    
    public boolean equals(JavaType jt){
        if (jt.theClass == this.theClass)
            return true;
        
        return false;
    }

    public String valueDesc() {
	if (theClass==null)
	    return "(null)";
	    
        return theClass.getName();
    }

}
